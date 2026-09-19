//! Native-path observation endpoint. No production storage or span semantics.

use std::sync::atomic::{AtomicU64, Ordering};
use tracing_core::{
    field::{Field, Visit},
    span,
    subscriber::Interest,
    Event, Level, LevelFilter, Metadata, Subscriber,
};

// Finite audit scenarios emit at most 1024 events; these are not production
// saturating loss counters. Observe them only after all emitting workers join.
static ACCEPTED: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);
static SEQUENCES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub accepted: u64,
    pub rejected: u64,
    pub sequences: u64,
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        accepted: ACCEPTED.load(Ordering::Relaxed),
        rejected: REJECTED.load(Ordering::Relaxed),
        sequences: SEQUENCES.load(Ordering::Relaxed),
    }
}

#[derive(Default)]
struct Fields {
    sequence: Option<u64>,
    rejected: bool,
}

impl Visit for Fields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "sequence" && value <= 1 && self.sequence.is_none() {
            self.sequence = Some(value);
        } else {
            self.rejected = true;
        }
    }

    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {
        self.rejected = true;
    }
}

pub struct AuditSubscriber;

impl Subscriber for AuditSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_event() && metadata.target() == "audit" && *metadata.level() <= Level::INFO
    }

    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::INFO)
    }

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        match fields.sequence {
            Some(sequence) if !fields.rejected => {
                ACCEPTED.fetch_add(1, Ordering::Relaxed);
                SEQUENCES.fetch_or(1 << sequence, Ordering::Relaxed);
            }
            _ => {
                REJECTED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        panic!("spans are outside this audit fixture")
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {
        panic!("spans are outside this audit fixture")
    }

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {
        panic!("spans are outside this audit fixture")
    }

    fn enter(&self, _: &span::Id) {
        panic!("spans are outside this audit fixture")
    }
    fn exit(&self, _: &span::Id) {
        panic!("spans are outside this audit fixture")
    }
}
