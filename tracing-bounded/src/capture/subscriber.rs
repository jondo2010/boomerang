//! Fixed-storage native subscriber. References are independent of capture locks;
//! a last release leaves a reclaimable slot if the span table is busy.

use super::{loss::Counter, *};
use std::{
    cell::Cell,
    marker::PhantomData,
    num::NonZeroU16,
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};
use tracing_core::{span, subscriber::Interest, LevelFilter, Metadata, Subscriber};

const INVALID: u64 = u64::MAX;
// Generation zero is never allocated. A filtered manual span behaves like a
// native disabled span: entering is transparent; an explicit parent is root.
const FILTERED: u64 = 1;
thread_local! { static PRODUCER: Cell<Option<(usize, usize)>> = const { Cell::new(None) }; }

/// Immutable capture limits. Setup allocates storage; callbacks never grow it.
#[derive(Clone, Debug)]
pub struct Config {
    /// Retained events; zero rejects every enabled event.
    pub records: usize,
    /// Total event and inherited field occurrences per record.
    pub fields: usize,
    /// Total copied event and inherited bytes per record.
    pub bytes: usize,
    /// Simultaneously live spans, including deferred reclamation.
    pub spans: usize,
    /// Declared fields per span.
    pub span_fields: usize,
    /// Copied value bytes per span.
    pub span_bytes: usize,
    /// Simultaneously prepared emitting threads.
    pub producers: usize,
    /// Maximum entered stack and parent ancestry depth.
    pub depth: usize,
    /// Maximum handles, entries and child references to one span.
    pub references: NonZeroU16,
    /// Saturation ceiling for each monotonic loss counter.
    pub loss_ceiling: NonZeroU16,
    /// Maximum enabled native tracing level.
    pub level: LevelFilter,
    /// Exact allowed targets; empty enables all targets at the chosen level.
    pub targets: &'static [&'static str],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            records: 64,
            fields: 32,
            bytes: 1024,
            spans: 64,
            span_fields: 16,
            span_bytes: 256,
            producers: 8,
            depth: 16,
            references: NonZeroU16::new(1024).unwrap(),
            loss_ceiling: NonZeroU16::new(1024).unwrap(),
            level: LevelFilter::INFO,
            targets: &[],
        }
    }
}

/// Native subscriber with preallocated event, span and producer storage.
/// Install explicitly using standard tracing dispatcher APIs. Every emitting
/// thread must hold a [`ProducerGuard`] before relying on capture.
pub struct BoundedSubscriber {
    state: Arc<State>,
}

/// Off-path inspection and producer preparation for one subscriber.
#[derive(Clone)]
pub struct CaptureHandle {
    state: Arc<State>,
}

/// Thread-bound admission guard. Drop releases entered references and the
/// producer slot. Span handles may outlive it and move to other prepared threads.
/// Preparation and teardown belong outside allocation-sensitive execution.
pub struct ProducerGuard {
    state: Arc<State>,
    index: usize,
    _not_send: PhantomData<Rc<()>>,
}

/// Producer preparation could not reserve a context.
#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    /// This thread already has a prepared subscriber context.
    AlreadyPrepared,
    /// Every configured producer slot is occupied.
    Capacity,
    /// Thread-local state is being destroyed.
    ThreadExiting,
}

/// Loss of span/context information, separate from event rejection counters.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LifecycleLoss {
    /// Failed creation, clone or reference admission.
    pub span_admission: LossCount,
    /// Failed span updates; affected spans remain invalid.
    pub span_update: LossCount,
    /// Failed producer preparation.
    pub producer_admission: LossCount,
    /// Producer contexts made persistently invalid until teardown.
    pub invalid_context_transitions: LossCount,
    /// Unsupported follows-from requests.
    pub unsupported_context_operation: LossCount,
}

struct State {
    config: Config,
    capture: Capture,
    inspector: Inspector,
    spans: Mutex<Spans>,
    refs: Box<[References]>,
    producers: Box<[Producer]>,
    admission: Counter,
    update: Counter,
    preparation: Counter,
    invalidations: Counter,
    unsupported: Counter,
}
struct References {
    generation: AtomicU32,
    count: AtomicU32,
    invalid: AtomicBool,
}
struct SpanSlot {
    values: Slot,
    parent: Option<u64>,
    live: bool,
    depth: usize,
}
struct Spans {
    slots: Box<[SpanSlot]>,
    scratch: Slot,
}
struct Producer {
    claimed: AtomicBool,
    invalid: AtomicBool,
    stack: Mutex<Stack>,
}
struct Stack {
    entries: Box<[Entry]>,
    len: usize,
}
#[derive(Clone, Copy, Default)]
struct Entry {
    id: u64,
    pinned: bool,
}

fn boxed<T>(
    count: usize,
    mut make: impl FnMut() -> Result<T, BuildError>,
) -> Result<Box<[T]>, BuildError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| BuildError::Allocation)?;
    for _ in 0..count {
        values.push(make()?);
    }
    Ok(values.into_boxed_slice())
}

fn prepared_mutex<T>(value: T) -> Mutex<T> {
    let mutex = Mutex::new(value);
    drop(mutex.lock().expect("new mutex"));
    mutex
}

impl BoundedSubscriber {
    /// Validate limits and allocate all callback storage. Shared `Arc` allocation
    /// and platform mutex setup may abort on global allocation failure; explicit
    /// array reservations return [`BuildError::Allocation`].
    pub fn new(config: Config) -> Result<(Self, CaptureHandle), BuildError> {
        if config.spans == 0
            || config.spans >= u32::MAX as usize
            || config.producers == 0
            || config.depth == 0
            || config.span_fields == 0
            || config.span_bytes == 0
        {
            return Err(BuildError::InvalidCapacity);
        }
        // Bound the aggregate requested element counts before any reservations.
        let span_units = config
            .span_fields
            .checked_mul(mem::size_of::<Option<record::StoredField>>())
            .and_then(|n| n.checked_add(config.span_bytes))
            .and_then(|n| n.checked_add(mem::size_of::<SpanSlot>() + mem::size_of::<References>()));
        let total = span_units
            .and_then(|n| n.checked_mul(config.spans.checked_add(1)?))
            .and_then(|n| {
                n.checked_add(
                    config.producers.checked_mul(
                        config
                            .depth
                            .checked_mul(mem::size_of::<Entry>())?
                            .checked_add(mem::size_of::<Producer>())?,
                    )?,
                )
            });
        if total.is_none_or(|n| n > isize::MAX as usize) {
            return Err(BuildError::SizeOverflow);
        }
        let (capture, inspector) = Capture::with_scopes(
            Limits {
                records: config.records,
                fields: config.fields,
                bytes: config.bytes,
                loss_ceiling: config.loss_ceiling,
            },
            config.depth,
        )?;
        let slots = boxed(config.spans, || {
            Ok(SpanSlot {
                values: Slot::new(config.span_fields, config.span_bytes)?,
                parent: None,
                live: false,
                depth: 0,
            })
        })?;
        let scratch = Slot::new(config.span_fields, config.span_bytes)?;
        let refs = boxed(config.spans, || {
            Ok(References {
                generation: AtomicU32::new(0),
                count: AtomicU32::new(0),
                invalid: AtomicBool::new(false),
            })
        })?;
        let producers = boxed(config.producers, || {
            Ok(Producer {
                claimed: AtomicBool::new(false),
                invalid: AtomicBool::new(false),
                stack: prepared_mutex(Stack {
                    entries: boxed(config.depth, || Ok(Entry::default()))?,
                    len: 0,
                }),
            })
        })?;
        let ceiling = config.loss_ceiling;
        let state = Arc::new(State {
            config,
            capture,
            inspector,
            spans: prepared_mutex(Spans { slots, scratch }),
            refs,
            producers,
            admission: Counter::new(ceiling),
            update: Counter::new(ceiling),
            preparation: Counter::new(ceiling),
            invalidations: Counter::new(ceiling),
            unsupported: Counter::new(ceiling),
        });
        Ok((
            Self {
                state: state.clone(),
            },
            CaptureHandle { state },
        ))
    }
}

impl State {
    fn token(&self) -> usize {
        std::ptr::from_ref(self) as usize
    }
    fn producer(&self) -> Option<&Producer> {
        let (token, index) = PRODUCER.try_with(Cell::get).ok().flatten()?;
        (token == self.token()).then(|| &self.producers[index])
    }
    fn invalidate(&self, producer: &Producer) {
        if !producer.invalid.swap(true, Ordering::Relaxed) {
            self.invalidations.increment();
        }
    }
    fn parent(&self, root: bool, explicit: Option<&span::Id>) -> Result<Option<u64>, Reject> {
        let producer = self.producer().ok_or(Reject::InvalidContext)?;
        if producer.invalid.load(Ordering::Relaxed) {
            return Err(Reject::InvalidContext);
        }
        if root {
            return Ok(None);
        }
        if let Some(id) = explicit {
            return Ok((id.into_u64() != FILTERED).then_some(id.into_u64()));
        }
        let stack = producer.stack.try_lock().map_err(|_| Reject::Contention)?;
        Ok(stack.len.checked_sub(1).map(|index| {
            let entry = stack.entries[index];
            if entry.pinned {
                entry.id
            } else {
                INVALID
            }
        }))
    }
    fn reference(&self, id: u64) -> Option<(usize, &References)> {
        let index = (id as u32).checked_sub(1)? as usize;
        let refs = self.refs.get(index)?;
        (refs.generation.load(Ordering::Acquire) == (id >> 32) as u32).then_some((index, refs))
    }
    // The caller owns a handle/entry/child pin while retaining. One strong CAS,
    // not a retry loop: contention fails admission without losing existing pins.
    fn retain(&self, id: u64) -> bool {
        let Some((_, refs)) = self.reference(id) else {
            return false;
        };
        let count = refs.count.load(Ordering::Relaxed);
        count > 0
            && count < u32::from(self.config.references.get())
            && refs
                .count
                .compare_exchange(count, count + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
    }
    // Every release consumes a previously owned pin; no lock and no retry.
    fn release(&self, id: u64) -> bool {
        self.reference(id)
            .is_some_and(|(_, refs)| refs.count.fetch_sub(1, Ordering::Release) == 1)
    }
    fn sweep(&self, spans: &mut Spans) {
        // At most S passes release a child->parent chain, including a reverse
        // slot order. No recursion, new allocation or waiting for another thread.
        for _ in 0..self.refs.len() {
            let mut changed = false;
            for (slot, refs) in spans.slots.iter_mut().zip(&self.refs) {
                if slot.live && refs.count.load(Ordering::Acquire) == 0 {
                    slot.live = false;
                    if let Some(parent) = slot.parent.take() {
                        self.release(parent);
                    }
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
    fn reclaim(&self) {
        if let Ok(mut spans) = self.spans.try_lock() {
            self.sweep(&mut spans);
        }
    }
    fn valid_index(&self, spans: &Spans, id: u64) -> Result<usize, Reject> {
        let (index, refs) = self.reference(id).ok_or(Reject::InvalidContext)?;
        if !spans.slots[index].live || refs.invalid.load(Ordering::Acquire) {
            return Err(Reject::InvalidContext);
        }
        Ok(index)
    }
    fn copy_context(&self, event: &Event<'_>, destination: &mut Slot) -> Result<(), Reject> {
        let Some(mut id) = self.parent(event.is_root(), event.parent())? else {
            return Ok(());
        };
        let mut spans = self.spans.try_lock().map_err(|_| Reject::Contention)?;
        // Use depth to walk from root to leaf without allocating a path vector.
        let leaf = self.valid_index(&spans, id)?;
        let depth = spans.slots[leaf].depth;
        let mut fields = event.metadata().fields().len();
        for _ in 0..depth {
            let index = self.valid_index(&spans, id)?;
            fields = fields.saturating_add(spans.slots[index].values.metadata().fields().len());
            if let Some(parent) = spans.slots[index].parent {
                id = parent;
            }
        }
        if fields > self.config.fields {
            return Err(Reject::FieldLimit);
        }
        for level in 1..=depth {
            let mut index = leaf;
            for _ in level..depth {
                index = self.valid_index(
                    &spans,
                    spans.slots[index].parent.ok_or(Reject::InvalidContext)?,
                )?;
            }
            destination.append_scope(&spans.slots[index].values)?;
        }
        self.sweep(&mut spans);
        Ok(())
    }
}

impl CaptureHandle {
    /// Admit this thread before allocation-sensitive execution. Each thread may
    /// prepare only one subscriber at a time. The guard is deliberately !Send.
    pub fn prepare_current_thread(&self) -> Result<ProducerGuard, PrepareError> {
        let result = PRODUCER
            .try_with(|current| {
                if current.get().is_some() {
                    return Err(PrepareError::AlreadyPrepared);
                }
                for (index, producer) in self.state.producers.iter().enumerate() {
                    if producer
                        .claimed
                        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                        .is_ok()
                    {
                        producer.invalid.store(false, Ordering::Relaxed);
                        current.set(Some((self.state.token(), index)));
                        return Ok(ProducerGuard {
                            state: self.state.clone(),
                            index,
                            _not_send: PhantomData,
                        });
                    }
                }
                Err(PrepareError::Capacity)
            })
            .unwrap_or(Err(PrepareError::ThreadExiting));
        if result.is_err() {
            self.state.preparation.increment();
        }
        result
    }
    /// Copy retained records off-path, in local commit order. May allocate/wait.
    pub fn snapshot(&self) -> Vec<Record> {
        self.state.inspector.snapshot()
    }
    /// Read out-of-band event losses without acquiring capture storage.
    pub fn loss(&self) -> LossSnapshot {
        self.state.inspector.loss()
    }
    /// Read out-of-band span and producer losses without acquiring storage.
    pub fn lifecycle_loss(&self) -> LifecycleLoss {
        LifecycleLoss {
            span_admission: self.state.admission.snapshot(),
            span_update: self.state.update.snapshot(),
            producer_admission: self.state.preparation.snapshot(),
            invalid_context_transitions: self.state.invalidations.snapshot(),
            unsupported_context_operation: self.state.unsupported.snapshot(),
        }
    }
    /// Whether this thread's prepared ancestry is healthy and immediately
    /// inspectable. Contention, unavailable ancestry and unprepared/torn-down
    /// contexts return false, even when their counters are zero.
    pub fn current_thread_is_valid(&self) -> bool {
        let Ok(mut parent) = self.state.parent(false, None) else {
            return false;
        };
        let Ok(spans) = self.state.spans.try_lock() else {
            return false;
        };
        for _ in 0..self.state.config.depth {
            let Some(id) = parent else {
                return true;
            };
            let Ok(index) = self.state.valid_index(&spans, id) else {
                return false;
            };
            parent = spans.slots[index].parent;
        }
        parent.is_none()
    }
}

impl Drop for ProducerGuard {
    fn drop(&mut self) {
        let _ = PRODUCER.try_with(|current| current.set(None));
        let producer = &self.state.producers[self.index];
        // Only this thread accesses its context; no user callbacks run while
        // this mutex is held, and inspection never locks it.
        if let Ok(mut stack) = producer.stack.try_lock() {
            for entry in &stack.entries[..stack.len] {
                if entry.pinned {
                    self.state.release(entry.id);
                }
            }
            stack.len = 0;
            producer.claimed.store(false, Ordering::Release);
        } else {
            self.state.invalidate(producer);
        }
        self.state.reclaim();
    }
}

impl Subscriber for BoundedSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= self.state.config.level
            && (self.state.config.targets.is_empty()
                || self.state.config.targets.contains(&metadata.target()))
    }
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }
    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(self.state.config.level)
    }
    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        self.state.capture.event_with(event, |destination| {
            self.state.copy_context(event, destination)
        });
    }
    fn new_span(&self, attributes: &span::Attributes<'_>) -> span::Id {
        if !self.enabled(attributes.metadata()) {
            return span::Id::from_u64(FILTERED);
        }
        let create = || -> Option<u64> {
            let parent = self
                .state
                .parent(attributes.is_root(), attributes.parent())
                .ok()?;
            let mut spans = self.state.spans.try_lock().ok()?;
            self.state.sweep(&mut spans);
            let depth = match parent {
                None => 1,
                Some(id) => spans.slots[self.state.valid_index(&spans, id).ok()?]
                    .depth
                    .checked_add(1)?,
            };
            if depth > self.state.config.depth
                || attributes.metadata().fields().len() > self.state.config.span_fields
            {
                return None;
            }
            let index = spans
                .slots
                .iter()
                .zip(&self.state.refs)
                .position(|(slot, refs)| {
                    !slot.live && refs.generation.load(Ordering::Relaxed) != u32::MAX
                })?;
            spans.scratch.begin(attributes.metadata());
            let mut visitor = spans.scratch.visitor();
            attributes.record(&mut visitor);
            if visitor.rejection().is_some() {
                return None;
            }
            if parent.is_some_and(|id| !self.state.retain(id)) {
                return None;
            }
            let Spans { slots, scratch } = &mut *spans;
            let slot = &mut slots[index];
            mem::swap(&mut slot.values, scratch);
            slot.parent = parent;
            slot.depth = depth;
            slot.live = true;
            let refs = &self.state.refs[index];
            let generation = refs.generation.load(Ordering::Relaxed) + 1;
            refs.invalid.store(false, Ordering::Relaxed);
            refs.count.store(1, Ordering::Relaxed);
            refs.generation.store(generation, Ordering::Release);
            Some((u64::from(generation) << 32) | (index as u64 + 1))
        };
        let id = create().unwrap_or_else(|| {
            self.state.admission.increment();
            INVALID
        });
        span::Id::from_u64(id)
    }
    fn record(&self, id: &span::Id, values: &span::Record<'_>) {
        if id.into_u64() == FILTERED {
            return;
        }
        let Some((index, refs)) = self.state.reference(id.into_u64()) else {
            self.state.update.increment();
            return;
        };
        let update = || -> Option<()> {
            if refs.invalid.load(Ordering::Acquire) {
                return None;
            }
            let mut spans = self.state.spans.try_lock().ok()?;
            let Spans { slots, scratch } = &mut *spans;
            let slot = &mut slots[index];
            scratch.begin(slot.values.metadata());
            let mut visitor = scratch.visitor();
            values.record(&mut visitor);
            if visitor.rejection().is_some() {
                return None;
            }
            scratch.retain_unmodified(&slot.values).ok()?;
            mem::swap(scratch, &mut slot.values);
            Some(())
        };
        if update().is_none() {
            refs.invalid.store(true, Ordering::Release);
            self.state.update.increment();
        }
        self.state.reclaim();
    }
    fn record_follows_from(&self, id: &span::Id, follows: &span::Id) {
        if id.into_u64() != FILTERED && follows.into_u64() != FILTERED {
            self.state.unsupported.increment();
        }
    }
    fn enter(&self, id: &span::Id) {
        if id.into_u64() == FILTERED {
            return;
        }
        let Some(producer) = self.state.producer() else {
            return;
        };
        if producer.invalid.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut stack) = producer.stack.try_lock() else {
            self.state.invalidate(producer);
            return;
        };
        if stack.len == stack.entries.len() {
            self.state.invalidate(producer);
            return;
        }
        let id = id.into_u64();
        let pinned = id != INVALID && self.state.retain(id);
        if id != INVALID && !pinned {
            self.state.admission.increment();
        }
        let index = stack.len;
        stack.entries[index] = Entry { id, pinned };
        stack.len += 1;
    }
    fn exit(&self, id: &span::Id) {
        if id.into_u64() == FILTERED {
            return;
        }
        let Some(producer) = self.state.producer() else {
            return;
        };
        if producer.invalid.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut stack) = producer.stack.try_lock() else {
            self.state.invalidate(producer);
            return;
        };
        let Some(index) = stack.len.checked_sub(1) else {
            self.state.invalidate(producer);
            return;
        };
        let entry = stack.entries[index];
        if entry.id != id.into_u64() {
            self.state.invalidate(producer);
            return;
        }
        stack.len = index;
        if entry.pinned {
            self.state.release(entry.id);
        }
        self.state.reclaim();
    }
    fn clone_span(&self, id: &span::Id) -> span::Id {
        if id.into_u64() == FILTERED {
            return id.clone();
        }
        if self.state.retain(id.into_u64()) {
            id.clone()
        } else {
            self.state.admission.increment();
            span::Id::from_u64(INVALID)
        }
    }
    fn try_close(&self, id: span::Id) -> bool {
        if id.into_u64() == INVALID || id.into_u64() == FILTERED {
            return false;
        }
        let last = self.state.release(id.into_u64());
        self.state.reclaim();
        last
    }
    fn current_span(&self) -> span::Current {
        let invalid = || span::Current::new(span::Id::from_u64(INVALID), &INVALID_METADATA);
        match self.state.parent(false, None) {
            Ok(None) => span::Current::none(),
            Ok(Some(id)) => {
                let Ok(spans) = self.state.spans.try_lock() else {
                    return invalid();
                };
                match self.state.valid_index(&spans, id) {
                    Ok(index) => span::Current::new(
                        span::Id::from_u64(id),
                        spans.slots[index].values.metadata(),
                    ),
                    Err(_) => invalid(),
                }
            }
            Err(_) => invalid(),
        }
    }
}

// An unavailable context must propagate an invalid handle, never Span::none(),
// which native callers could mistake for an explicit healthy root.
static INVALID_CALLSITE: tracing_core::callsite::DefaultCallsite =
    tracing_core::callsite::DefaultCallsite::new(&INVALID_METADATA);
static INVALID_METADATA: Metadata<'static> = Metadata::new(
    "unavailable context",
    "tracing_bounded",
    tracing_core::Level::ERROR,
    None,
    None,
    None,
    tracing_core::field::FieldSet::new(&[], tracing_core::callsite::Identifier(&INVALID_CALLSITE)),
    tracing_core::metadata::Kind::SPAN,
);

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ZeroFields => "field capacity must be positive",
            Self::ZeroBytes => "byte capacity must be positive",
            Self::SizeOverflow => "storage size overflow",
            Self::Allocation => "storage reservation failed",
            Self::InvalidCapacity => "invalid span or producer capacity",
        })
    }
}
impl std::error::Error for BuildError {}
impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AlreadyPrepared => "thread already prepared",
            Self::Capacity => "producer capacity exhausted",
            Self::ThreadExiting => "thread local state unavailable",
        })
    }
}
impl std::error::Error for PrepareError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_close_under_contention_keeps_ancestry_and_reclaims_before_reuse() {
        let (subscriber, handle) = BoundedSubscriber::new(Config {
            spans: 2,
            ..Config::default()
        })
        .unwrap();
        let _producer = handle.prepare_current_thread().unwrap();
        tracing::subscriber::with_default(subscriber, || {
            let parent = tracing::info_span!("parent", key = 1u64);
            let child = tracing::info_span!(parent: &parent, "child", key = 2u64);
            let held = handle.state.spans.lock().unwrap();
            let copy = child.clone();
            drop(parent);
            drop(child);
            drop(held);
            copy.in_scope(|| tracing::info!(ok = true));
            let held = handle.state.spans.lock().unwrap();
            drop(copy);
            drop(held);
            let reused = tracing::info_span!("reused", key = 3u64);
            reused.in_scope(|| tracing::info!(ok = true));
        });
        let records = handle.snapshot();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].scopes[0].metadata.name(), "parent");
        assert_eq!(records[1].scopes[0].metadata.name(), "reused");
        assert_eq!(handle.lifecycle_loss().span_admission.value, 0);
    }

    #[test]
    fn contended_update_invalidates_descendants_without_erasing_old_output() {
        let (subscriber, handle) = BoundedSubscriber::new(Config::default()).unwrap();
        let _producer = handle.prepare_current_thread().unwrap();
        tracing::subscriber::with_default(subscriber, || {
            let parent = tracing::info_span!("parent", key = 1u64);
            let child = tracing::info_span!(parent: &parent, "child");
            child.in_scope(|| tracing::info!(ok = true));
            let held = handle.state.spans.lock().unwrap();
            parent.record("key", 2u64);
            drop(held);
            child.in_scope(|| tracing::info!(ok = false));
        });
        assert_eq!(handle.snapshot().len(), 1);
        assert_eq!(
            handle.snapshot()[0].scopes[0].fields[0].value,
            Value::U64(1)
        );
        assert_eq!(handle.lifecycle_loss().span_update.value, 1);
        assert_eq!(handle.loss().invalid_context.value, 1);
    }

    #[test]
    fn exhausted_generation_retires_slot_without_aliasing_retained_identity() {
        let (subscriber, handle) = BoundedSubscriber::new(Config {
            spans: 1,
            ..Config::default()
        })
        .unwrap();
        handle.state.refs[0]
            .generation
            .store(u32::MAX - 1, Ordering::Relaxed);
        let _producer = handle.prepare_current_thread().unwrap();
        tracing::subscriber::with_default(subscriber, || {
            let last = tracing::info_span!("last");
            last.in_scope(|| tracing::info!(ok = true));
            drop(last);
            let retired = tracing::info_span!("retired");
            retired.in_scope(|| tracing::info!(ok = false));
        });
        assert_eq!(handle.snapshot().len(), 1);
        assert_eq!(handle.snapshot()[0].scopes[0].metadata.name(), "last");
        assert_eq!(handle.lifecycle_loss().span_admission.value, 1);
        assert_eq!(handle.loss().invalid_context.value, 1);
    }
}
