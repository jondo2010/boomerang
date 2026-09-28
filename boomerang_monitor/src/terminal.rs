use std::{future::Future, io, pin::pin, time::Instant};

use boomerang_telemetry::MAX_DATAGRAM_BYTES;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_util::{Stream, StreamExt};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Row, Table, TableState},
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

/// Source selection and plot choice independent of telemetry receiver state.
#[derive(Debug, Default)]
pub struct DashboardState {
    selected: usize,
    selected_identity: Option<crate::SourceIdentitySnapshot>,
    plot: PlotKind,
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
    render_sources(frame, panes[0], snapshot, state);
    render_selected(frame, panes[1], snapshot, state);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
            Span::raw(" source  "),
            Span::styled("1", Style::default().fg(Color::Cyan)),
            Span::raw(" throughput  "),
            Span::styled("2", Style::default().fg(Color::Cyan)),
            Span::raw(" scheduler time  "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(" quit"),
        ])),
        outer[1],
    );
}

fn render_sources(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    snapshot: &MonitorSnapshot,
    state: &DashboardState,
) {
    let rows = snapshot.sources.iter().map(|source| {
        let scheduler = source.scheduler.as_ref();
        let state = scheduler
            .map(|scheduler| format!("{:?}", scheduler.latest.raw.lifecycle))
            .unwrap_or_else(|| "unavailable".into());
        let freshness = freshness_label(classify_freshness(
            scheduler.and_then(|scheduler| scheduler.sequence.age),
        ));
        let gaps = scheduler.map_or(0, |scheduler| scheduler.sequence.skipped);
        Row::new([
            source_label(&source.identity),
            state,
            freshness.into(),
            gaps.to_string(),
        ])
    });
    let header = Row::new(["Source", "State", "Scheduler", "Gap"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(42),
            Constraint::Percentage(24),
            Constraint::Percentage(22),
            Constraint::Percentage(12),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        "Sources {}  accepted {} rejected {}",
        snapshot.sources.len(),
        snapshot.counters.accepted,
        snapshot.counters.rejected()
    )))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");
    let mut table_state =
        TableState::default().with_selected(state.selected_index(&snapshot.sources));
    frame.render_stateful_widget(table, area, &mut table_state);
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
            render_details(frame, parts[0], source);
            render_plot(frame, parts[1], source, state.plot());
        }
        SelectedPresentation::PlotOnly if source.scheduler.is_some() => {
            render_plot(frame, area, source, state.plot());
        }
        SelectedPresentation::PlotOnly | SelectedPresentation::DetailsOnly => {
            render_details(frame, area, source);
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

fn render_details(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    source: &crate::SourceSnapshot,
) {
    frame.render_widget(
        Paragraph::new(detail_lines(source)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Selected: {}", source_label(&source.identity))),
        ),
        area,
    );
}

fn detail_lines(source: &crate::SourceSnapshot) -> Vec<Line<'static>> {
    let Some(scheduler) = source.scheduler.as_ref() else {
        return vec![
            Line::from("scheduler: unavailable"),
            exporter_line(source.exporter_health.as_ref()),
        ];
    };
    let raw = &scheduler.latest.raw;
    let age = duration_label(scheduler.sequence.age);
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
        Line::from(format!(
            "{:?}  {:?} {phase_elapsed}  |  {} age {age}",
            raw.lifecycle,
            raw.current_phase,
            freshness_label(classify_freshness(scheduler.sequence.age))
        )),
        Line::from(format!(
            "tags {}  progress {progress} ago  |  sequence {}",
            raw.completed_logical_tags, scheduler.latest.sequence
        )),
        Line::from(format!(
            "queue {}  peak {}  reserved {}  enforced limit {}",
            raw.event_queue_occupancy,
            raw.event_queue_peak_occupancy,
            raw.event_queue_reserved_capacity,
            raw.event_queue_enforced_limit
                .map_or_else(|| "unknown".into(), |value| value.to_string())
        )),
        Line::from(format!(
            "skipped {}  reordered {}  discontinuities {}  evicted {}",
            scheduler.sequence.skipped,
            scheduler.sequence.stale_or_reordered,
            scheduler.sequence.discontinuities,
            scheduler.history_evictions
        )),
        exporter_line(source.exporter_health.as_ref()),
    ]
}

fn exporter_line(exporter: Option<&crate::ExporterHealthSnapshot>) -> Line<'static> {
    exporter.map_or_else(
        || Line::from("exporter health: unavailable"),
        |exporter| {
            Line::from(format!(
                "export drops {}  misses {}  |  {} age {}",
                exporter.latest.raw.publication_drops,
                exporter.latest.raw.snapshot_misses,
                freshness_label(classify_freshness(exporter.sequence.age)),
                duration_label(exporter.sequence.age)
            ))
        },
    )
}

fn render_plot(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    source: &crate::SourceSnapshot,
    plot: PlotKind,
) {
    let Some(scheduler) = source.scheduler.as_ref() else {
        return;
    };
    let history = &scheduler.history;
    let x_max = history.len().saturating_sub(1).max(1) as f64;
    let (metric_title, series): (&str, Vec<PlotSeries<'_>>) = match plot {
        PlotKind::Throughput => (
            "Throughput (operations/s)",
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
            "Scheduler-accounted time (ms/s, not OS CPU)",
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
    let title = format!("{} — {}", source_label(&source.identity), metric_title);
    let y_max = series
        .iter()
        .flat_map(|(_, _, points)| points.iter().map(|(_, y)| *y))
        .fold(1.0_f64, f64::max);
    let datasets = series
        .iter()
        .map(|(name, color, points)| {
            Dataset::default()
                .name(*name)
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(*color))
                .data(points)
        })
        .collect::<Vec<_>>();
    let chart = Chart::new(datasets)
        .block(Block::default().borders(Borders::ALL).title(title))
        .x_axis(Axis::default().bounds([0.0, x_max]))
        .y_axis(Axis::default().bounds([0.0, y_max * 1.05]));
    frame.render_widget(chart, area);
}

fn rate_points(
    history: &[crate::SchedulerSample],
    rate: impl Fn(&crate::SchedulerRates) -> Option<u64>,
    scale: f64,
) -> Vec<(f64, f64)> {
    history
        .iter()
        .enumerate()
        .filter_map(|(index, sample)| {
            rate(&sample.rates).map(|value| (index as f64, value as f64 / scale))
        })
        .collect()
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
        classify_freshness, run_dashboard_with, selected_presentation, with_terminal_lifecycle,
        DashboardAction, DashboardState, Freshness, PlotKind, SelectedPresentation,
    };
    use crate::{MonitorOptions, ReceiverConfig, SourceIdentitySnapshot, SourceSnapshot};
    use boomerang_telemetry::{
        ExporterHealth, RecordGroup, SourceIdentity, SourceRole, TelemetryRecord, TelemetryValue,
        MAX_DATAGRAM_BYTES,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};
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
