use std::{
    num::NonZeroU16,
    sync::atomic::{AtomicU32, Ordering},
};

use super::{LossCount, LossSnapshot, Reject};

pub(super) struct Counter {
    value: AtomicU32,
    ceiling: NonZeroU16,
}

pub(super) struct Loss {
    no_record_capacity: Counter,
    contention: Counter,
    invalid_context: Counter,
    field_limit: Counter,
    byte_limit: Counter,
    unsupported_value: Counter,
    overwritten_records: Counter,
}

impl Counter {
    pub(super) fn new(ceiling: NonZeroU16) -> Self {
        Self {
            value: AtomicU32::new(0),
            ceiling,
        }
    }

    // A deployment may choose a smaller ceiling to reduce this source-level
    // work bound at the cost of earlier saturation. The maximum is 65,535 CAS
    // attempts; this does not claim a hardware retry bound, timing bound, or
    // hard WCET.
    pub(super) fn increment(&self) {
        let mut seen = self.value.load(Ordering::Relaxed);
        for _ in 0..self.ceiling.get() {
            if seen == u32::from(self.ceiling.get()) {
                return;
            }
            match self
                .value
                .compare_exchange(seen, seen + 1, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(actual) => seen = actual,
            }
        }
    }

    pub(super) fn snapshot(&self) -> LossCount {
        let value = self.value.load(Ordering::Relaxed);
        LossCount {
            value,
            saturated: value == u32::from(self.ceiling.get()),
        }
    }
}

impl Loss {
    pub(super) fn new(ceiling: NonZeroU16) -> Self {
        Self {
            no_record_capacity: Counter::new(ceiling),
            contention: Counter::new(ceiling),
            invalid_context: Counter::new(ceiling),
            field_limit: Counter::new(ceiling),
            byte_limit: Counter::new(ceiling),
            unsupported_value: Counter::new(ceiling),
            overwritten_records: Counter::new(ceiling),
        }
    }

    pub(super) fn increment(&self, reject: Reject) {
        match reject {
            Reject::NoRecordCapacity => &self.no_record_capacity,
            Reject::Contention => &self.contention,
            Reject::InvalidContext => &self.invalid_context,
            Reject::FieldLimit => &self.field_limit,
            Reject::ByteLimit => &self.byte_limit,
            Reject::UnsupportedValue => &self.unsupported_value,
        }
        .increment();
    }

    pub(super) fn overwrite(&self) {
        self.overwritten_records.increment();
    }

    pub(super) fn snapshot(&self) -> LossSnapshot {
        LossSnapshot {
            no_record_capacity: self.no_record_capacity.snapshot(),
            contention: self.contention.snapshot(),
            invalid_context: self.invalid_context.snapshot(),
            field_limit: self.field_limit.snapshot(),
            byte_limit: self.byte_limit.snapshot(),
            unsupported_value: self.unsupported_value.snapshot(),
            overwritten_records: self.overwritten_records.snapshot(),
        }
    }
}
