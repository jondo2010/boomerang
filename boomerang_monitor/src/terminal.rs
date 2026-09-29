use std::{future::Future, io, pin::pin, time::Instant};

use boomerang_telemetry::MAX_DATAGRAM_BYTES;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_util::{Stream, StreamExt};
use ratatui::{
    backend::Backend,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Row, StatefulWidget, Table,
        TableState, Widget,
    },
    Terminal,
};
use tokio::{net::UdpSocket, time::MissedTickBehavior};

use crate::{IngestOutcome, MonitorError, MonitorOptions, MonitorSnapshot, Receiver};

const STALE_AFTER: std::time::Duration = std::time::Duration::from_millis(500);
const DISCONNECTED_AFTER: std::time::Duration = std::time::Duration::from_secs(2);
type PlotSeries<'a> = (&'a str, Color, Vec<(f64, f64)>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Freshness {
    Live,
    Stale,
    Disconnected,
    Unavailable,
}

fn classify_freshness(age: Option<std::time::Duration>) -> Freshness {
    match age {
        None => Freshness::Unavailable,
        Some(age) if age <= STALE_AFTER => Freshness::Live,
        Some(age) if age <= DISCONNECTED_AFTER => Freshness::Stale,
        Some(_) => Freshness::Disconnected,
    }
}

fn with_terminal_lifecycle<T, R, E>(
    init: impl FnOnce() -> Result<T, E>,
    run: impl FnOnce(&mut T) -> Result<R, E>,
    restore: impl FnOnce(),
) -> Result<R, E> {
    match init() {
        Ok(mut terminal) => {
            let result = run(&mut terminal);
            restore();
            result
        }
        Err(error) => {
            restore();
            Err(error)
        }
    }
}

/// Run the live hosted terminal dashboard until the configured receiver limit,
/// idle timeout, keyboard exit, or process interruption is observed.
pub fn serve_dashboard(options: &MonitorOptions) -> Result<MonitorSnapshot, MonitorError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(MonitorError::Terminal)?;
    with_terminal_lifecycle(
        || ratatui::try_init().map_err(MonitorError::Terminal),
        |terminal| {
            runtime.block_on(run_dashboard_with(
                options,
                terminal,
                crossterm::event::EventStream::new(),
                tokio::signal::ctrl_c(),
            ))
        },
        ratatui::restore,
    )
}

/// Historical series shown for the selected telemetry source.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PlotKind {
    /// Reactions, events, and completed logical tags per second.
    #[default]
    Throughput,
    /// Scheduler-accounted callback, framework, and wait time in milliseconds per second.
    SchedulerTime,
}

/// Result of applying one terminal input event to dashboard state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DashboardAction {
    /// Continue receiving telemetry and rendering frames.
    Continue,
    /// Restore the terminal and return the latest snapshot.
    Exit,
}

/// Metric columns shown in the source overview table.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TableMode {
    /// Lifecycle, scheduler freshness, and sequence integrity.
    #[default]
    Status,
    /// Receiver-derived scheduler throughput rates.
    Activity,
    /// Event-queue occupancy, peak, and enforced limit.
    Queue,
    /// Exporter loss counters and freshness.
    Exporter,
}

impl TableMode {
    fn next(self) -> Self {
        match self {
            Self::Status => Self::Activity,
            Self::Activity => Self::Queue,
            Self::Queue => Self::Exporter,
            Self::Exporter => Self::Status,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Status => Self::Exporter,
            Self::Activity => Self::Status,
            Self::Queue => Self::Activity,
            Self::Exporter => Self::Queue,
        }
    }
}

/// Source selection and plot choice independent of telemetry receiver state.
#[derive(Debug, Default)]
pub struct DashboardState {
    selected: usize,
    selected_identity: Option<crate::SourceIdentitySnapshot>,
    plot: PlotKind,
    table_mode: TableMode,
}

impl DashboardState {
    /// Selected source index after reconciling the stable source identity.
    pub fn selected_index(&self, sources: &[crate::SourceSnapshot]) -> Option<usize> {
        self.selected_identity
            .as_ref()
            .and_then(|identity| {
                sources
                    .iter()
                    .position(|source| &source.identity == identity)
            })
            .or_else(|| (!sources.is_empty()).then(|| self.selected.min(sources.len() - 1)))
    }

    /// Preserve the selected source across deterministic snapshot reordering.
    pub fn sync_sources(&mut self, sources: &[crate::SourceSnapshot]) {
        let Some(index) = self.selected_index(sources) else {
            self.selected = 0;
            self.selected_identity = None;
            return;
        };
        self.selected = index;
        self.selected_identity = Some(sources[index].identity.clone());
    }

    /// Historical series currently selected for display.
    pub fn plot(&self) -> PlotKind {
        self.plot
    }

    /// Metric columns currently shown in the source overview table.
    pub fn table_mode(&self) -> TableMode {
        self.table_mode
    }

    /// Apply one key event without modifying receiver-owned telemetry state.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        sources: &[crate::SourceSnapshot],
    ) -> DashboardAction {
        match (key.code, key.modifiers) {
            (KeyCode::Char('q') | KeyCode::Esc, _)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                return DashboardAction::Exit;
            }
            (KeyCode::Down | KeyCode::Char('j'), _) if !sources.is_empty() => {
                self.selected = (self.selected + 1).min(sources.len() - 1);
                self.selected_identity = Some(sources[self.selected].identity.clone());
            }
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                self.selected = self.selected.saturating_sub(1);
                self.selected_identity = sources
                    .get(self.selected)
                    .map(|source| source.identity.clone());
            }
            (KeyCode::Char('1'), _) => self.plot = PlotKind::Throughput,
            (KeyCode::Char('2'), _) => self.plot = PlotKind::SchedulerTime,
            (KeyCode::Tab, _) => self.table_mode = self.table_mode.next(),
            (KeyCode::BackTab, _) => self.table_mode = self.table_mode.previous(),
            _ => {}
        }
        DashboardAction::Continue
    }
}

pub(crate) async fn run_dashboard_with<B, E, S>(
    options: &MonitorOptions,
    terminal: &mut Terminal<B>,
    mut events: E,
    shutdown: S,
) -> Result<MonitorSnapshot, MonitorError>
where
    B: Backend,
    E: Stream<Item = io::Result<crossterm::event::Event>> + Unpin,
    S: Future<Output = io::Result<()>>,
{
    let socket = UdpSocket::bind(options.listen)
        .await
        .map_err(MonitorError::Bind)?;
    let origin = Instant::now();
    let mut receiver = Receiver::new(options.receiver);
    let mut state = DashboardState::default();
    let mut buffer = [0; MAX_DATAGRAM_BYTES + 1];
    let mut scratch = [0; MAX_DATAGRAM_BYTES];
    let mut accepted = 0_usize;
    let mut refresh = tokio::time::interval(std::time::Duration::from_millis(250));
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let idle_duration = options
        .idle_timeout
        .unwrap_or(std::time::Duration::from_secs(100 * 365 * 24 * 60 * 60));
    let idle = tokio::time::sleep(idle_duration);
    let mut idle = pin!(idle);
    let mut shutdown = pin!(shutdown);
    let mut events_open = true;

    loop {
        tokio::select! {
            received = socket.recv_from(&mut buffer) => {
                match received {
                    Ok((length, _sender)) => {
                        let received_at = origin.elapsed();
                        if matches!(receiver.ingest_with_scratch(&buffer[..length], received_at, &mut scratch), IngestOutcome::Accepted) {
                            accepted = accepted.saturating_add(1);
                            if options.max_records.is_some_and(|limit| accepted >= limit.get()) {
                                return Ok(receiver.snapshot(received_at));
                            }
                        }
                        if options.idle_timeout.is_some() {
                            idle.as_mut().reset(tokio::time::Instant::now() + idle_duration);
                        }
                    }
                    #[cfg(windows)]
                    Err(error) if error.raw_os_error() == Some(10040) => receiver.record_oversized_datagram(),
                    Err(error) => return Err(MonitorError::Receive(error)),
                }
            }
            _ = refresh.tick() => {
                let snapshot = receiver.snapshot(origin.elapsed());
                state.sync_sources(&snapshot.sources);
                terminal.draw(|frame| render(frame, &snapshot, &state)).map_err(MonitorError::Terminal)?;
            }
            event = events.next(), if events_open => {
                match event {
                    Some(Ok(crossterm::event::Event::Key(key))) => {
                        let snapshot = receiver.snapshot(origin.elapsed());
                        state.sync_sources(&snapshot.sources);
                        if matches!(state.handle_key(key, &snapshot.sources), DashboardAction::Exit) {
                            return Ok(snapshot);
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(MonitorError::TerminalInput(error)),
                    None => events_open = false,
                }
            }
            result = &mut shutdown => {
                result.map_err(MonitorError::TerminalInput)?;
                return Ok(receiver.snapshot(origin.elapsed()));
            }
            _ = &mut idle, if options.idle_timeout.is_some() => {
                let counters = receiver.snapshot(origin.elapsed()).counters;
                return Err(MonitorError::IdleTimeout { accepted: counters.accepted, rejected: counters.rejected() });
            }
        }
    }
}

fn render(frame: &mut ratatui::Frame<'_>, snapshot: &MonitorSnapshot, state: &DashboardState) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(outer[0]);
    let mut table_state =
        TableState::default().with_selected(state.selected_index(&snapshot.sources));
    frame.render_stateful_widget(
        SourceOverview::new(snapshot, state.table_mode()),
        panes[0],
        &mut table_state,
    );
    render_selected(frame, panes[1], snapshot, state);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
            Span::raw(" source  "),
            Span::styled("1", Style::default().fg(Color::Cyan)),
            Span::raw(" throughput  "),
            Span::styled("2", Style::default().fg(Color::Cyan)),
            Span::raw(" scheduler time  "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::raw(" table  "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(" quit"),
        ])),
        outer[1],
    );
}

struct SourceOverview<'a> {
    snapshot: &'a MonitorSnapshot,
    mode: TableMode,
}

impl<'a> SourceOverview<'a> {
    fn new(snapshot: &'a MonitorSnapshot, mode: TableMode) -> Self {
        Self { snapshot, mode }
    }
}

impl StatefulWidget for SourceOverview<'_> {
    type State = TableState;

    fn render(self, area: Rect, buffer: &mut Buffer, state: &mut Self::State) {
        let (headers, widths) = table_columns(self.mode);
        let rows = self.snapshot.sources.iter().map(|source| {
            let alert = source_has_alert(source);
            let row = Row::new(source_row(source, self.mode));
            if alert {
                row.style(Style::default().fg(Color::Red))
            } else {
                row
            }
        });
        let header = Row::new(headers).style(Style::default().add_modifier(Modifier::BOLD));
        let title = if area.width < 60 {
            format!(
                "Sources {} · {}",
                self.snapshot.sources.len(),
                table_mode_label(self.mode)
            )
        } else {
            format!(
                "Sources {} · {} · accepted {} rejected {}",
                self.snapshot.sources.len(),
                table_mode_label(self.mode),
                self.snapshot.counters.accepted,
                self.snapshot.counters.rejected()
            )
        };
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(title))
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        StatefulWidget::render(table, area, buffer, state);
    }
}

fn table_mode_label(mode: TableMode) -> &'static str {
    match mode {
        TableMode::Status => "Status",
        TableMode::Activity => "Activity",
        TableMode::Queue => "Queue",
        TableMode::Exporter => "Exporter",
    }
}

fn table_columns(mode: TableMode) -> (Vec<&'static str>, Vec<Constraint>) {
    let headers = match mode {
        TableMode::Status => vec!["", "Source", "State", "Health", "Seq"],
        TableMode::Activity => vec!["", "Source", "Rxn/s", "Evt/s", "Tag/s"],
        TableMode::Queue => vec!["", "Source", "Used", "Peak", "Limit"],
        TableMode::Exporter => vec!["", "Source", "Drops", "Miss", "Age"],
    };
    (
        headers,
        vec![
            Constraint::Length(1),
            Constraint::Percentage(40),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
}

fn source_row(source: &crate::SourceSnapshot, mode: TableMode) -> Vec<String> {
    let scheduler = source.scheduler.as_ref();
    let exporter = source.exporter_health.as_ref();
    let mut row = vec![
        if source_has_alert(source) { "!" } else { "" }.into(),
        compact_source_label(&source.identity),
    ];
    match mode {
        TableMode::Status => row.extend([
            scheduler
                .map(|scheduler| format!("{:?}", scheduler.latest.raw.lifecycle))
                .unwrap_or_else(|| "unavailable".into()),
            freshness_label(classify_freshness(
                scheduler.and_then(|scheduler| scheduler.sequence.age),
            ))
            .into(),
            scheduler.map_or_else(
                || "-".into(),
                |scheduler| {
                    format!(
                        "{}/{}",
                        scheduler.sequence.skipped, scheduler.sequence.discontinuities
                    )
                },
            ),
        ]),
        TableMode::Activity => row.extend([
            scheduler
                .and_then(|value| value.latest.rates.processed_reactions_per_second)
                .map_or_else(|| "-".into(), |value| value.to_string()),
            scheduler
                .and_then(|value| value.latest.rates.processed_events_per_second)
                .map_or_else(|| "-".into(), |value| value.to_string()),
            scheduler
                .and_then(|value| value.latest.rates.completed_logical_tags_per_second)
                .map_or_else(|| "-".into(), |value| value.to_string()),
        ]),
        TableMode::Queue => row.extend([
            scheduler.map_or_else(
                || "-".into(),
                |value| value.latest.raw.event_queue_occupancy.to_string(),
            ),
            scheduler.map_or_else(
                || "-".into(),
                |value| value.latest.raw.event_queue_peak_occupancy.to_string(),
            ),
            scheduler
                .and_then(|value| value.latest.raw.event_queue_enforced_limit)
                .map_or_else(|| "?".into(), |value| value.to_string()),
        ]),
        TableMode::Exporter => row.extend([
            exporter.map_or_else(
                || "-".into(),
                |value| value.latest.raw.publication_drops.to_string(),
            ),
            exporter.map_or_else(
                || "-".into(),
                |value| value.latest.raw.snapshot_misses.to_string(),
            ),
            duration_label(exporter.and_then(|value| value.sequence.age)),
        ]),
    }
    row
}

fn source_has_alert(source: &crate::SourceSnapshot) -> bool {
    let scheduler_alert = source.scheduler.as_ref().is_some_and(|scheduler| {
        matches!(
            classify_freshness(scheduler.sequence.age),
            Freshness::Stale | Freshness::Disconnected
        ) || scheduler.sequence.skipped > 0
            || scheduler.sequence.stale_or_reordered > 0
            || scheduler.sequence.discontinuities > 0
    });
    let exporter_alert = source.exporter_health.as_ref().is_some_and(|exporter| {
        matches!(
            classify_freshness(exporter.sequence.age),
            Freshness::Stale | Freshness::Disconnected
        ) || exporter.sequence.skipped > 0
            || exporter.sequence.stale_or_reordered > 0
            || exporter.latest.raw.publication_drops > 0
            || exporter.latest.raw.snapshot_misses > 0
    });
    scheduler_alert || exporter_alert
}

fn render_selected(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    snapshot: &MonitorSnapshot,
    state: &DashboardState,
) {
    let Some(source) = state
        .selected_index(&snapshot.sources)
        .and_then(|index| snapshot.sources.get(index))
    else {
        frame.render_widget(
            Paragraph::new("Waiting for telemetry").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Selected source"),
            ),
            area,
        );
        return;
    };
    match selected_presentation(area) {
        SelectedPresentation::DetailsAndPlot => {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(7), Constraint::Min(0)])
                .split(area);
            frame.render_widget(SelectedDetails::new(source), parts[0]);
            frame.render_widget(TelemetryPlot::new(source, state.plot()), parts[1]);
        }
        SelectedPresentation::PlotOnly if source.scheduler.is_some() => {
            frame.render_widget(TelemetryPlot::new(source, state.plot()), area);
        }
        SelectedPresentation::PlotOnly | SelectedPresentation::DetailsOnly => {
            frame.render_widget(SelectedDetails::new(source), area);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectedPresentation {
    DetailsAndPlot,
    PlotOnly,
    DetailsOnly,
}

fn selected_presentation(area: ratatui::layout::Rect) -> SelectedPresentation {
    if area.height >= 11 && area.width >= 48 {
        SelectedPresentation::DetailsAndPlot
    } else if area.height >= 5 && area.width >= 32 {
        SelectedPresentation::PlotOnly
    } else {
        SelectedPresentation::DetailsOnly
    }
}

struct SelectedDetails<'a> {
    source: &'a crate::SourceSnapshot,
}

impl<'a> SelectedDetails<'a> {
    fn new(source: &'a crate::SourceSnapshot) -> Self {
        Self { source }
    }
}

impl Widget for SelectedDetails<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        Paragraph::new(detail_lines(self.source))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Selected: {}", source_label(&self.source.identity))),
            )
            .render(area, buffer);
    }
}

fn detail_lines(source: &crate::SourceSnapshot) -> Vec<Line<'static>> {
    detail_rows(source).into_iter().map(detail_line).collect()
}

#[derive(Debug, Eq, PartialEq)]
struct DetailItem {
    label: &'static str,
    value: String,
}

impl DetailItem {
    fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct DetailRow {
    label: &'static str,
    items: Vec<DetailItem>,
}

impl DetailRow {
    fn new(label: &'static str, items: Vec<DetailItem>) -> Self {
        Self { label, items }
    }
}

fn detail_rows(source: &crate::SourceSnapshot) -> Vec<DetailRow> {
    let scheduler = source.scheduler.as_ref();
    let exporter = source.exporter_health.as_ref();
    let runtime = scheduler.map_or_else(
        || vec![DetailItem::new("state", "unavailable")],
        |scheduler| {
            let raw = &scheduler.latest.raw;
            vec![
                DetailItem::new("state", format!("{:?}", raw.lifecycle)),
                DetailItem::new("phase", compact_phase_label(raw.current_phase)),
                DetailItem::new(
                    "fresh",
                    format!(
                        "{} {}",
                        freshness_label(classify_freshness(scheduler.sequence.age)),
                        duration_label(scheduler.sequence.age)
                    ),
                ),
            ]
        },
    );
    let progress = scheduler.map_or_else(
        || vec![DetailItem::new("tags", "unavailable")],
        |scheduler| {
            let raw = &scheduler.latest.raw;
            let progress = raw
                .last_logical_progress_ns
                .map(|last| {
                    scheduler
                        .latest
                        .observation_monotonic_ns
                        .saturating_sub(last)
                })
                .map(nanoseconds_label)
                .unwrap_or_else(|| "unknown".into());
            let phase_elapsed = nanoseconds_label(
                scheduler
                    .latest
                    .observation_monotonic_ns
                    .saturating_sub(raw.current_phase_started_ns),
            );
            vec![
                DetailItem::new("tags", raw.completed_logical_tags.to_string()),
                DetailItem::new("last", progress),
                DetailItem::new("sequence", scheduler.latest.sequence.to_string()),
                DetailItem::new("phase", phase_elapsed),
            ]
        },
    );
    let queue = scheduler.map_or_else(
        || vec![DetailItem::new("used", "unavailable")],
        |scheduler| {
            let raw = &scheduler.latest.raw;
            vec![
                DetailItem::new("used", raw.event_queue_occupancy.to_string()),
                DetailItem::new("peak", raw.event_queue_peak_occupancy.to_string()),
                DetailItem::new("reserved", raw.event_queue_reserved_capacity.to_string()),
                DetailItem::new(
                    "limit",
                    raw.event_queue_enforced_limit
                        .map_or_else(|| "unknown".into(), |value| value.to_string()),
                ),
            ]
        },
    );
    let exporter_status = exporter.map_or_else(
        || vec![DetailItem::new("fresh", "unavailable")],
        |exporter| {
            let freshness = classify_freshness(exporter.sequence.age);
            vec![DetailItem::new(
                "fresh",
                if freshness == Freshness::Live {
                    freshness_label(freshness).into()
                } else {
                    format!(
                        "{} {}",
                        freshness_label(freshness),
                        duration_label(exporter.sequence.age)
                    )
                },
            )]
        },
    );
    let errors = vec![
        DetailItem::new(
            "sched s/r/d/e",
            scheduler.map_or_else(
                || "unavailable".into(),
                |value| {
                    format!(
                        "{}/{}/{}/{}",
                        compact_counter(value.sequence.skipped),
                        compact_counter(value.sequence.stale_or_reordered),
                        compact_counter(value.sequence.discontinuities),
                        compact_counter(value.history_evictions)
                    )
                },
            ),
        ),
        DetailItem::new(
            "export s/r/d/m",
            exporter.map_or_else(
                || "unavailable".into(),
                |value| {
                    format!(
                        "{}/{}/{}/{}",
                        compact_counter(value.sequence.skipped),
                        compact_counter(value.sequence.stale_or_reordered),
                        compact_counter(value.latest.raw.publication_drops),
                        compact_counter(value.latest.raw.snapshot_misses)
                    )
                },
            ),
        ),
    ];
    vec![
        DetailRow::new("Runtime", runtime),
        DetailRow::new("Progress", progress),
        DetailRow::new("Queue", queue),
        DetailRow::new("Exporter", exporter_status),
        DetailRow::new("Errors", errors),
    ]
}

fn compact_phase_label(phase: impl std::fmt::Debug) -> String {
    let label = format!("{phase:?}");
    label.strip_suffix("Wait").unwrap_or(&label).to_owned()
}

fn compact_counter(mut value: u64) -> String {
    const UNITS: [&str; 7] = ["", "k", "M", "G", "T", "P", "E"];
    let mut unit = 0;
    while unit + 1 < UNITS.len() && value >= 1_000 {
        value = value / 1_000 + u64::from(value % 1_000 >= 500);
        unit += 1;
    }
    if unit > 0 && unit + 1 < UNITS.len() && value >= 100 {
        let tenths = value / 100 + u64::from(value % 100 >= 50);
        return if tenths >= 10 {
            format!("1{}", UNITS[unit + 1])
        } else {
            format!(".{tenths}{}", UNITS[unit + 1])
        };
    }
    format!("{value}{}", UNITS[unit])
}

fn detail_line(row: DetailRow) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{:<8}", row.label),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    for (index, item) in row.items.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("│"));
        }
        spans.push(Span::styled(
            format!("{} ", item.label),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(item.value));
    }
    Line::from(spans)
}

struct TelemetryPlot<'a> {
    source: &'a crate::SourceSnapshot,
    plot: PlotKind,
}

impl<'a> TelemetryPlot<'a> {
    fn new(source: &'a crate::SourceSnapshot, plot: PlotKind) -> Self {
        Self { source, plot }
    }
}

impl Widget for TelemetryPlot<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let Some(scheduler) = self.source.scheduler.as_ref() else {
            return;
        };
        let history = &scheduler.history;
        let latest_observation_ns = history
            .last()
            .map_or(0, |sample| sample.observation_monotonic_ns);
        let x_min = history
            .first()
            .map_or(-1.0, |sample| {
                relative_observation_seconds(latest_observation_ns, sample.observation_monotonic_ns)
            })
            .min(-1.0);
        let (metric_title, y_unit, series): (&str, &str, Vec<PlotSeries<'_>>) = match self.plot {
            PlotKind::Throughput => (
                "Throughput",
                "ops/s",
                vec![
                    (
                        "reactions",
                        Color::Cyan,
                        rate_points(history, |rates| rates.processed_reactions_per_second, 1.0),
                    ),
                    (
                        "events",
                        Color::Yellow,
                        rate_points(history, |rates| rates.processed_events_per_second, 1.0),
                    ),
                    (
                        "tags",
                        Color::Green,
                        rate_points(
                            history,
                            |rates| rates.completed_logical_tags_per_second,
                            1.0,
                        ),
                    ),
                ],
            ),
            PlotKind::SchedulerTime => (
                "Scheduler time (not OS CPU)",
                "ms/s",
                vec![
                    (
                        "reaction",
                        Color::Cyan,
                        rate_points(
                            history,
                            |rates| rates.reaction_elapsed_ns_per_second,
                            1_000_000.0,
                        ),
                    ),
                    (
                        "framework",
                        Color::Magenta,
                        rate_points(
                            history,
                            |rates| rates.framework_elapsed_ns_per_second,
                            1_000_000.0,
                        ),
                    ),
                    (
                        "physical wait",
                        Color::Blue,
                        rate_points(
                            history,
                            |rates| rates.physical_wait_elapsed_ns_per_second,
                            1_000_000.0,
                        ),
                    ),
                    (
                        "external wait",
                        Color::Yellow,
                        rate_points(
                            history,
                            |rates| rates.external_wait_elapsed_ns_per_second,
                            1_000_000.0,
                        ),
                    ),
                    (
                        "coordination wait",
                        Color::Green,
                        rate_points(
                            history,
                            |rates| rates.coordination_wait_elapsed_ns_per_second,
                            1_000_000.0,
                        ),
                    ),
                ],
            ),
        };
        let title = if area.width < 64 {
            format!(
                "{} — {metric_title} [{y_unit}]",
                compact_source_label(&self.source.identity)
            )
        } else {
            format!(
                "{} — {metric_title} [{y_unit}]",
                source_label(&self.source.identity)
            )
        };
        let y_max = series
            .iter()
            .flat_map(|(_, _, points)| points.iter().map(|(_, y)| *y))
            .fold(1.0_f64, f64::max);
        let y_upper = y_max.ceil().max(1.0);
        let datasets = series
            .iter()
            .map(|(_name, color, points)| {
                Dataset::default()
                    .marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Scatter)
                    .style(Style::default().fg(*color))
                    .data(points)
            })
            .collect::<Vec<_>>();
        let x_labels = vec![Span::raw(format!("{x_min:.0}s")), Span::raw("latest")];
        let y_labels = vec![
            Span::raw("0"),
            Span::raw(format_value(y_upper / 2.0)),
            Span::raw(format_value(y_upper)),
        ];
        let chart = Chart::new(datasets)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_bottom(plot_legend(self.plot)),
            )
            .x_axis(
                Axis::default()
                    .title("source time")
                    .bounds([x_min, 0.0])
                    .labels(x_labels),
            )
            .y_axis(
                Axis::default()
                    .title(y_unit)
                    .bounds([0.0, y_upper])
                    .labels(y_labels),
            );
        chart.render(area, buffer);
    }
}

fn plot_legend(plot: PlotKind) -> Line<'static> {
    let entries: &[(&str, Color)] = match plot {
        PlotKind::Throughput => &[
            ("R rxn", Color::Cyan),
            ("E evt", Color::Yellow),
            ("T tag", Color::Green),
        ],
        PlotKind::SchedulerTime => &[
            ("R react", Color::Cyan),
            ("F fw", Color::Magenta),
            ("P phys", Color::Blue),
            ("E ext", Color::Yellow),
            ("C coord", Color::Green),
        ],
    };
    Line::from(
        entries
            .iter()
            .enumerate()
            .flat_map(|(index, (label, color))| {
                let separator = (index > 0).then(|| Span::raw("  "));
                separator.into_iter().chain(std::iter::once(Span::styled(
                    (*label).to_owned(),
                    Style::default().fg(*color),
                )))
            })
            .collect::<Vec<_>>(),
    )
    .alignment(Alignment::Right)
}

fn format_value(value: f64) -> String {
    if value >= 10.0 || value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn rate_points(
    history: &[crate::SchedulerSample],
    rate: impl Fn(&crate::SchedulerRates) -> Option<u64>,
    scale: f64,
) -> Vec<(f64, f64)> {
    let latest_observation_ns = history
        .last()
        .map_or(0, |sample| sample.observation_monotonic_ns);
    history
        .iter()
        .filter_map(|sample| {
            rate(&sample.rates).map(|value| {
                (
                    relative_observation_seconds(
                        latest_observation_ns,
                        sample.observation_monotonic_ns,
                    ),
                    value as f64 / scale,
                )
            })
        })
        .collect()
}

fn relative_observation_seconds(latest_ns: u64, observation_ns: u64) -> f64 {
    -(latest_ns.saturating_sub(observation_ns) as f64 / 1_000_000_000.0)
}

fn source_label(identity: &crate::SourceIdentitySnapshot) -> String {
    format!(
        "{:?}:{}/{}@{}#{}",
        identity.role,
        identity.federate_id.as_deref().unwrap_or("unknown"),
        identity.enclave_id.as_deref().unwrap_or("unknown"),
        identity.process_id,
        identity.process_incarnation
    )
}

fn compact_source_label(identity: &crate::SourceIdentitySnapshot) -> String {
    format!(
        "{}/{}@{}#{}",
        identity.federate_id.as_deref().unwrap_or("?"),
        identity.enclave_id.as_deref().unwrap_or("?"),
        identity.process_id,
        identity.process_incarnation
    )
}

fn freshness_label(freshness: Freshness) -> &'static str {
    match freshness {
        Freshness::Live => "live",
        Freshness::Stale => "stale",
        Freshness::Disconnected => "disconnected",
        Freshness::Unavailable => "unavailable",
    }
}

fn duration_label(duration: Option<std::time::Duration>) -> String {
    duration.map_or_else(
        || "unknown".into(),
        |duration| {
            if duration.as_secs() > 0 {
                format!("{:.1}s", duration.as_secs_f64())
            } else {
                format!("{}ms", duration.as_millis())
            }
        },
    )
}

fn nanoseconds_label(nanoseconds: u64) -> String {
    duration_label(Some(std::time::Duration::from_nanos(nanoseconds)))
}

#[cfg(test)]
mod tests {
    use super::{
        classify_freshness, detail_rows, run_dashboard_with, selected_presentation,
        with_terminal_lifecycle, DashboardAction, DashboardState, Freshness, PlotKind,
        SelectedDetails, SelectedPresentation, SourceOverview, TableMode, TelemetryPlot,
    };
    use crate::{MonitorOptions, Receiver, ReceiverConfig, SourceIdentitySnapshot, SourceSnapshot};
    use boomerang_telemetry::{
        ExporterHealth, RecordGroup, SourceIdentity, SourceRole, TelemetryRecord, TelemetryValue,
        MAX_DATAGRAM_BYTES,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{
        backend::TestBackend,
        widgets::{StatefulWidget, TableState, Widget},
        Terminal,
    };
    use std::{
        net::UdpSocket,
        num::NonZeroUsize,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    fn source(process_id: &str) -> SourceSnapshot {
        SourceSnapshot {
            identity: SourceIdentitySnapshot {
                run_id: [1; 16],
                artifact_id: [2; 32],
                process_id: process_id.into(),
                process_incarnation: 1,
                role: SourceRole::Federate,
                federate_id: Some("host".into()),
                enclave_id: Some("sensor".into()),
            },
            scheduler: None,
            exporter_health: None,
        }
    }

    fn detail_source() -> SourceSnapshot {
        let raw = serde_json::from_value(serde_json::json!({
            "lifecycle": "Running", "current_phase": "Framework",
            "current_phase_started_ns": 42,
            "reaction_elapsed_ns": 1, "framework_elapsed_ns": 2,
            "physical_wait_elapsed_ns": 3, "external_wait_elapsed_ns": 4,
            "coordination_wait_elapsed_ns": 5, "processed_tags": 6,
            "processed_reactions": 7, "processed_events": 8,
            "set_ports": 9, "scheduled_actions": 10,
            "event_queue_occupancy": 11, "event_queue_reserved_capacity": 12,
            "event_queue_enforced_limit": null, "event_queue_peak_occupancy": 13,
            "completed_logical_tags": 14, "last_logical_progress_ns": 23
        }))
        .unwrap();
        let mut receiver = Receiver::new(ReceiverConfig::default());
        for record in [
            TelemetryRecord {
                protocol_version: 1,
                group: RecordGroup::Scheduler,
                group_sequence: 0,
                run_id: [1; 16],
                artifact_id: [2; 32],
                process_id: "worker",
                process_incarnation: 3,
                source: SourceIdentity {
                    role: SourceRole::Federate,
                    federate_id: Some("host"),
                    enclave_id: Some("sensor"),
                },
                sender_monotonic_ns: 100,
                observation_monotonic_ns: 90,
                value: TelemetryValue::Scheduler(raw),
            },
            TelemetryRecord {
                protocol_version: 1,
                group: RecordGroup::ExporterHealth,
                group_sequence: 0,
                run_id: [1; 16],
                artifact_id: [2; 32],
                process_id: "worker",
                process_incarnation: 3,
                source: SourceIdentity {
                    role: SourceRole::Federate,
                    federate_id: Some("host"),
                    enclave_id: Some("sensor"),
                },
                sender_monotonic_ns: 100,
                observation_monotonic_ns: 90,
                value: TelemetryValue::ExporterHealth(ExporterHealth {
                    publication_drops: 2,
                    snapshot_misses: 3,
                }),
            },
            TelemetryRecord {
                protocol_version: 1,
                group: RecordGroup::ExporterHealth,
                group_sequence: 0,
                run_id: [1; 16],
                artifact_id: [2; 32],
                process_id: "worker",
                process_incarnation: 3,
                source: SourceIdentity {
                    role: SourceRole::Federate,
                    federate_id: Some("host"),
                    enclave_id: Some("sensor"),
                },
                sender_monotonic_ns: 100,
                observation_monotonic_ns: 90,
                value: TelemetryValue::ExporterHealth(ExporterHealth {
                    publication_drops: 99,
                    snapshot_misses: 99,
                }),
            },
        ] {
            let mut bytes = [0; MAX_DATAGRAM_BYTES];
            let length = record.encode_into(&mut bytes).unwrap();
            receiver.ingest(&bytes[..length], Duration::ZERO);
        }
        receiver
            .snapshot(Duration::from_millis(100))
            .sources
            .remove(0)
    }

    #[test]
    fn dashboard_pieces_use_ratatui_widget_contracts() {
        fn assert_widget<T: Widget>() {}
        fn assert_stateful_widget<T: StatefulWidget<State = TableState>>() {}

        assert_stateful_widget::<SourceOverview<'_>>();
        assert_widget::<SelectedDetails<'_>>();
        assert_widget::<TelemetryPlot<'_>>();
    }

    #[test]
    fn details_group_loss_counters_separately_and_show_live_age_once() {
        let rows = detail_rows(&detail_source());

        assert_eq!(
            rows.iter().map(|row| row.label).collect::<Vec<_>>(),
            ["Runtime", "Progress", "Queue", "Exporter", "Errors"]
        );
        assert_eq!(
            rows.iter()
                .map(|row| { row.items.iter().map(|item| item.label).collect::<Vec<_>>() })
                .collect::<Vec<_>>(),
            [
                vec!["state", "phase", "fresh"],
                vec!["tags", "last", "sequence", "phase"],
                vec!["used", "peak", "reserved", "limit"],
                vec!["fresh"],
                vec!["sched s/r/d/e", "export s/r/d/m"],
            ]
        );
        assert_eq!(rows[0].items[2].value, "live 100ms");
        assert_eq!(rows[3].items[0].value, "live");
        assert_eq!(rows[4].items[0].value, "0/0/0/0");
        assert_eq!(rows[4].items[1].value, "0/1/2/3");
    }

    #[test]
    fn compact_error_counters_have_a_three_column_ceiling() {
        assert_eq!(super::compact_counter(999), "999");
        assert_eq!(super::compact_counter(1_000), "1k");
        assert_eq!(super::compact_counter(100_000), ".1M");
        assert_eq!(super::compact_counter(499_999), ".5M");
        assert_eq!(super::compact_counter(999_999), "1M");
        assert_eq!(super::compact_counter(1_000_000), "1M");
        assert_eq!(super::compact_counter(u64::MAX), "18E");
    }

    #[test]
    fn navigation_and_plot_keys_change_only_dashboard_presentation_state() {
        let mut state = DashboardState::default();
        let sources = vec![source("a"), source("b"), source("c")];

        assert_eq!(state.selected_index(&sources), Some(0));
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &sources,),
            DashboardAction::Continue
        );
        assert_eq!(state.selected_index(&sources), Some(1));
        assert_eq!(
            state.handle_key(
                KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
                &sources,
            ),
            DashboardAction::Continue
        );
        assert_eq!(state.plot(), PlotKind::SchedulerTime);
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &sources,),
            DashboardAction::Continue
        );
        assert_eq!(state.selected_index(&sources), Some(0));
    }

    #[test]
    fn quit_keys_request_exit_without_sources() {
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut state = DashboardState::default();
            assert_eq!(state.handle_key(key, &[]), DashboardAction::Exit);
        }
    }

    #[test]
    fn tab_cycles_source_table_modes_and_backtab_reverses() {
        let mut state = DashboardState::default();

        assert_eq!(state.table_mode(), TableMode::Status);
        for expected in [
            TableMode::Activity,
            TableMode::Queue,
            TableMode::Exporter,
            TableMode::Status,
        ] {
            assert_eq!(
                state.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &[]),
                DashboardAction::Continue
            );
            assert_eq!(state.table_mode(), expected);
        }
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), &[]),
            DashboardAction::Continue
        );
        assert_eq!(state.table_mode(), TableMode::Exporter);
    }

    #[test]
    fn selected_source_identity_survives_an_earlier_sorted_insertion() {
        let mut state = DashboardState::default();
        let initial = vec![source("b"), source("c")];
        state.sync_sources(&initial);
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &initial);

        let inserted = vec![source("a"), source("b"), source("c")];
        state.sync_sources(&inserted);

        assert_eq!(state.selected_index(&inserted), Some(2));
    }

    #[test]
    fn shallow_selected_pane_prioritizes_the_active_plot() {
        assert_eq!(
            selected_presentation(ratatui::layout::Rect::new(0, 0, 46, 7)),
            SelectedPresentation::PlotOnly
        );
    }

    #[test]
    fn plot_points_use_source_observation_seconds_relative_to_latest() {
        assert_eq!(
            super::relative_observation_seconds(12_000_000_000, 10_000_000_000),
            -2.0
        );
        assert_eq!(
            super::relative_observation_seconds(12_000_000_000, 12_000_000_000),
            0.0
        );
    }

    #[tokio::test]
    async fn asynchronous_dashboard_ingests_udp_until_the_finite_record_limit() {
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let listen = probe.local_addr().unwrap();
        drop(probe);
        let options = MonitorOptions {
            listen,
            json: false,
            max_records: Some(NonZeroUsize::new(1).unwrap()),
            idle_timeout: Some(Duration::from_secs(2)),
            receiver: ReceiverConfig::default(),
        };
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let record = TelemetryRecord {
                protocol_version: 1,
                group: RecordGroup::ExporterHealth,
                group_sequence: 0,
                run_id: [1; 16],
                artifact_id: [2; 32],
                process_id: "worker",
                process_incarnation: 3,
                source: SourceIdentity {
                    role: SourceRole::Federate,
                    federate_id: Some("host"),
                    enclave_id: Some("sensor"),
                },
                sender_monotonic_ns: 20,
                observation_monotonic_ns: 10,
                value: TelemetryValue::ExporterHealth(ExporterHealth {
                    publication_drops: 0,
                    snapshot_misses: 0,
                }),
            };
            let mut bytes = [0; MAX_DATAGRAM_BYTES];
            let length = record.encode_into(&mut bytes).unwrap();
            UdpSocket::bind("127.0.0.1:0")
                .unwrap()
                .send_to(&bytes[..length], listen)
                .unwrap();
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();

        let snapshot = run_dashboard_with(
            &options,
            &mut terminal,
            futures_util::stream::pending(),
            std::future::pending::<std::io::Result<()>>(),
        )
        .await
        .unwrap();

        sender.join().unwrap();
        assert_eq!(snapshot.counters.accepted, 1);
        assert_eq!(snapshot.sources.len(), 1);
    }

    #[tokio::test]
    async fn ended_terminal_event_stream_is_not_polled_repeatedly() {
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let listen = probe.local_addr().unwrap();
        drop(probe);
        let options = MonitorOptions {
            listen,
            json: false,
            max_records: None,
            idle_timeout: None,
            receiver: ReceiverConfig::default(),
        };
        let polls = Arc::new(AtomicUsize::new(0));
        let stream_polls = Arc::clone(&polls);
        let events = futures_util::stream::poll_fn(move |_| {
            stream_polls.fetch_add(1, Ordering::Relaxed);
            std::task::Poll::Ready(None)
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();

        run_dashboard_with(&options, &mut terminal, events, async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(polls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn terminal_lifecycle_restores_after_a_handled_error() {
        let mut restored = false;
        let result: Result<(), &str> = with_terminal_lifecycle(
            || Ok::<_, &str>(()),
            |_| Err("draw failed"),
            || restored = true,
        );

        assert_eq!(result, Err("draw failed"));
        assert!(restored);
    }

    #[test]
    fn terminal_lifecycle_attempts_restoration_after_initialization_failure() {
        let mut restored = false;
        let result: Result<(), &str> = with_terminal_lifecycle(
            || Err("initialization failed"),
            |_: &mut ()| Ok(()),
            || restored = true,
        );

        assert_eq!(result, Err("initialization failed"));
        assert!(restored);
    }

    #[test]
    fn freshness_distinguishes_unavailable_stale_and_disconnected_sources() {
        assert_eq!(classify_freshness(None), Freshness::Unavailable);
        assert_eq!(
            classify_freshness(Some(Duration::from_millis(500))),
            Freshness::Live
        );
        assert_eq!(
            classify_freshness(Some(Duration::from_millis(501))),
            Freshness::Stale
        );
        assert_eq!(
            classify_freshness(Some(Duration::from_secs(2))),
            Freshness::Stale
        );
        assert_eq!(
            classify_freshness(Some(Duration::from_millis(2001))),
            Freshness::Disconnected
        );
    }
}
