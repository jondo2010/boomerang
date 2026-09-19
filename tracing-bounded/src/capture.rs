use std::{
    mem,
    num::NonZeroU16,
    sync::{Arc, Mutex},
};

use tracing_core::Event;

mod loss;
mod record;

use loss::Loss;
use record::{Record, Slot};

#[cfg(test)]
mod tests;

pub(super) struct Limits {
    pub(super) records: usize,
    pub(super) fields: usize,
    pub(super) bytes: usize,
    pub(super) loss_ceiling: NonZeroU16,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BuildError {
    ZeroFields,
    ZeroBytes,
    SizeOverflow,
    Allocation,
}

#[derive(Clone, Copy)]
pub(super) enum Reject {
    NoRecordCapacity,
    Contention,
    InvalidContext,
    FieldLimit,
    ByteLimit,
    UnsupportedValue,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct LossCount {
    pub(super) value: u32,
    pub(super) saturated: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct LossSnapshot {
    pub(super) no_record_capacity: LossCount,
    pub(super) contention: LossCount,
    pub(super) invalid_context: LossCount,
    pub(super) field_limit: LossCount,
    pub(super) byte_limit: LossCount,
    pub(super) unsupported_value: LossCount,
    pub(super) overwritten_records: LossCount,
}

pub(super) struct Capture {
    shared: Arc<Shared>,
}

#[derive(Clone)]
pub(super) struct Inspector {
    shared: Arc<Shared>,
}

struct Shared {
    limits: Limits,
    storage: Mutex<Storage>,
    loss: Loss,
}

struct Storage {
    ring: Box<[Slot]>,
    scratch: Slot,
    oldest: usize,
    len: usize,
}

impl Capture {
    pub(super) fn new(limits: Limits) -> Result<(Self, Inspector), BuildError> {
        if limits.fields == 0 {
            return Err(BuildError::ZeroFields);
        }
        if limits.bytes == 0 {
            return Err(BuildError::ZeroBytes);
        }
        let slots = limits
            .records
            .checked_add(1)
            .ok_or(BuildError::SizeOverflow)?;
        let total_fields = slots
            .checked_mul(limits.fields)
            .ok_or(BuildError::SizeOverflow)?;
        let total_bytes = slots
            .checked_mul(limits.bytes)
            .ok_or(BuildError::SizeOverflow)?;
        let slot_bytes = slots
            .checked_mul(mem::size_of::<Slot>())
            .ok_or(BuildError::SizeOverflow)?;
        let field_bytes = total_fields
            .checked_mul(mem::size_of::<Option<record::StoredField>>())
            .ok_or(BuildError::SizeOverflow)?;
        let total_layout = slot_bytes
            .checked_add(field_bytes)
            .and_then(|layout| layout.checked_add(total_bytes))
            .ok_or(BuildError::SizeOverflow)?;
        if total_layout > isize::MAX as usize {
            return Err(BuildError::SizeOverflow);
        }

        let mut ring = Vec::new();
        ring.try_reserve_exact(limits.records)
            .map_err(|_| BuildError::Allocation)?;
        for _ in 0..limits.records {
            ring.push(Slot::new(limits.fields, limits.bytes)?);
        }
        let scratch = Slot::new(limits.fields, limits.bytes)?;

        let shared = Arc::new(Shared {
            loss: Loss::new(limits.loss_ceiling),
            limits,
            storage: Mutex::new(Storage {
                ring: ring.into_boxed_slice(),
                scratch,
                oldest: 0,
                len: 0,
            }),
        });
        // Some targets lazily allocate platform mutex storage on first lock.
        // Complete that setup before either handle can reach the capture path.
        drop(shared.storage.lock().expect("new mutex cannot be poisoned"));
        Ok((
            Self {
                shared: Arc::clone(&shared),
            },
            Inspector { shared },
        ))
    }

    pub(super) fn event(&self, event: &Event<'_>) {
        if self.shared.limits.records == 0 {
            self.shared.loss.increment(Reject::NoRecordCapacity);
            return;
        }

        let Ok(mut storage) = self.shared.storage.try_lock() else {
            self.shared.loss.increment(Reject::Contention);
            return;
        };

        if !event.is_root() {
            self.shared.loss.increment(Reject::InvalidContext);
            return;
        }
        // Internal work bound: native macro metadata and value sets match, so
        // visitation cannot exceed this count. Manually constructed ValueSets
        // can repeat fields; their visitation work is not yet qualified.
        if event.metadata().fields().len() > self.shared.limits.fields {
            self.shared.loss.increment(Reject::FieldLimit);
            return;
        }

        storage.scratch.begin(event.metadata());
        let rejection = {
            let mut visitor = storage.scratch.visitor();
            event.record(&mut visitor);
            visitor.rejection()
        };
        if let Some(rejection) = rejection {
            self.shared.loss.increment(rejection);
            return;
        }

        let capacity = storage.ring.len();
        let destination = if storage.len == capacity {
            storage.oldest
        } else {
            wrapping_index(storage.oldest, storage.len, capacity)
        };
        let Storage { ring, scratch, .. } = &mut *storage;
        mem::swap(scratch, &mut ring[destination]);
        if storage.len == capacity {
            storage.oldest = wrapping_index(storage.oldest, 1, capacity);
            self.shared.loss.overwrite();
        } else {
            storage.len += 1;
        }
    }
}

impl Inspector {
    pub(super) fn loss(&self) -> LossSnapshot {
        self.shared.loss.snapshot()
    }

    fn snapshot(&self) -> Vec<Record> {
        let records = {
            let storage = self
                .shared
                .storage
                .lock()
                .expect("capture storage mutex must not be poisoned");
            (0..storage.len)
                .map(|offset| {
                    let index = wrapping_index(storage.oldest, offset, storage.ring.len());
                    storage.ring[index].snapshot()
                })
                .collect()
        };
        records
    }
}

fn wrapping_index(base: usize, offset: usize, capacity: usize) -> usize {
    let tail = capacity - base;
    if offset < tail {
        base + offset
    } else {
        offset - tail
    }
}

/// Catch lazy capture-path allocation without warming up the storage or callsite.
#[cfg(test)]
#[test]
#[cfg_attr(miri, ignore = "run alone under Miri with --ignored --exact")]
fn first_backend_event_does_not_allocate() {
    use crate::test_allocation::{self as allocation, Counts};
    use record::{OwnedField, Value};

    const CHILD: &str = "TRACING_BOUNDED_FIRST_EVENT_CHILD";
    const COMPLETE: &str = "backend-first-use complete";
    if !cfg!(miri) && std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "capture::first_backend_event_does_not_allocate",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .expect("start fresh first-event test process");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "first-event child failed: {}\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        // A stale exact filter must not silently pass by selecting zero tests.
        assert!(
            stdout.contains(COMPLETE),
            "child did not complete the probe: {stdout}"
        );
        return;
    }

    allocation::prepare();
    assert_eq!(allocation::measure(|| {}), Counts::default());
    let control = allocation::measure(|| drop(std::hint::black_box(Box::new(42_u64))));
    assert!(
        control.allocations > 0 && control.deallocations > 0,
        "observer missed allocation/deallocation: {control:?}"
    );

    let (capture, inspect) = Capture::new(Limits {
        records: 1,
        fields: 2,
        bytes: 2,
        loss_ceiling: NonZeroU16::new(10).unwrap(),
    })
    .expect("valid probe limits");
    tracing::subscriber::set_global_default(tests::CaptureSubscriber { capture })
        .expect("one fixed global subscriber");
    allocation::prepare();

    // No preceding event, snapshot, or probe access to the storage mutex.
    let counts = allocation::measure(|| {
        tracing::info!(target: "capture-test", parent: None, n = 42_u64, text = "ok");
    });
    let records = inspect.snapshot();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].metadata.target(), "capture-test");
    assert_eq!(*records[0].metadata.level(), tracing_core::Level::INFO);
    assert_eq!(
        records[0].fields,
        [
            OwnedField {
                name: "n",
                value: Value::U64(42)
            },
            OwnedField {
                name: "text",
                value: Value::Str("ok".into())
            },
        ]
    );
    assert_eq!(inspect.loss(), LossSnapshot::default());
    assert_eq!(
        counts,
        Counts::default(),
        "first backend emission allocated/deallocated"
    );
    println!("{COMPLETE}: accepted=1; fields=n:42,text:ok; loss=0; {counts:?}");
}
