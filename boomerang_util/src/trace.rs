//! Bounded retention of the bytes emitted by a standard `tracing_subscriber::fmt` subscriber.
//!
//! Pass a clone of [`TraceRing`](crate::trace::TraceRing) to the subscriber's `with_writer` method and retain the original
//! for inspection. Recording uses only preallocated slots and never waits for another writer
//! or a snapshot reader. Oldest records are overwritten when full; oversized and contended
//! records are discarded in full. [`TraceRing::dropped_records`](crate::trace::TraceRing::dropped_records) counts all three cases.
//!
//! These are retention bounds, not bounds on the subscriber's own formatting buffers. Select
//! a formatter appropriate to the platform and record only bounded fields. An absent or
//! disabled subscriber provides off mode; the standard stderr/file writer provides streaming.
//!
//! ```
//! use boomerang_util::trace::TraceRing;
//! let ring = TraceRing::new(64, 1024);
//! let subscriber = tracing_subscriber::fmt().json()
//!     .with_writer(ring.clone()).finish();
//! tracing::subscriber::with_default(subscriber, || tracing::info!(event = "ready"));
//! assert_eq!(ring.records().len(), 1);
//! assert_eq!(ring.dropped_records(), 0);
//! ```

use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

/// Shared, fixed-capacity retention for formatted trace records.
#[derive(Clone)]
pub struct TraceRing(Arc<Shared>);

struct Shared {
    ring: Mutex<Records>,
    dropped: AtomicUsize,
}

struct Records {
    slots: Vec<Vec<u8>>,
    lengths: Vec<usize>,
    scratch: Vec<u8>,
    next: usize,
    retained: usize,
}

impl TraceRing {
    /// Allocates `capacity` record slots and one scratch slot of `max_record_bytes` each.
    /// Zero capacity or zero record size drops every record with loss accounting.
    pub fn new(capacity: usize, max_record_bytes: usize) -> Self {
        Self(Arc::new(Shared {
            ring: Mutex::new(Records {
                slots: vec![vec![0; max_record_bytes]; capacity],
                lengths: vec![0; capacity],
                scratch: vec![0; max_record_bytes],
                next: 0,
                retained: 0,
            }),
            dropped: AtomicUsize::new(0),
        }))
    }

    /// Copies retained complete records in oldest-to-newest order.
    /// This reader may wait for a writer; producers never wait for this reader.
    pub fn records(&self) -> Vec<Vec<u8>> {
        let ring = self
            .0
            .ring
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (0..ring.retained)
            .map(|offset| {
                let first = if ring.retained == ring.slots.len() {
                    ring.next
                } else {
                    0
                };
                let index = (first + offset) % ring.slots.len();
                ring.slots[index][..ring.lengths[index]].to_vec()
            })
            .collect()
    }

    /// Returns records lost through overwriting, oversize, zero capacity, or contention.
    pub fn dropped_records(&self) -> usize {
        self.0.dropped.load(Ordering::Relaxed)
    }
}

/// One formatter write transaction; dropping it commits or discards the entire record.
pub struct TraceRecordWriter<'a> {
    ring: Option<MutexGuard<'a, Records>>,
    dropped: &'a AtomicUsize,
    written: usize,
    overflow: bool,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TraceRing {
    type Writer = TraceRecordWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        TraceRecordWriter {
            ring: self.0.ring.try_lock().ok(),
            dropped: &self.0.dropped,
            written: 0,
            overflow: false,
        }
    }
}

impl io::Write for TraceRecordWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(ring) = &mut self.ring {
            if !self.overflow && bytes.len() <= ring.scratch.len() - self.written {
                ring.scratch[self.written..self.written + bytes.len()].copy_from_slice(bytes);
                self.written += bytes.len();
            } else {
                self.overflow = true;
            }
        }
        // Loss is reported by the counter, never as an I/O failure in the application.
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for TraceRecordWriter<'_> {
    fn drop(&mut self) {
        let Some(ring) = self.ring.as_deref_mut() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if self.overflow || ring.slots.is_empty() || ring.scratch.is_empty() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let index = ring.next;
        std::mem::swap(&mut ring.slots[index], &mut ring.scratch);
        ring.lengths[index] = self.written;
        ring.next = (index + 1) % ring.slots.len();
        if ring.retained == ring.slots.len() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        } else {
            ring.retained += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contended_and_zero_capacity_subscriber_output_counts_whole_record_loss() {
        for capacity in [0, 1] {
            let ring = TraceRing::new(capacity, 512);
            let subscriber = tracing_subscriber::fmt()
                .json()
                .without_time()
                .with_writer(ring.clone())
                .finish();
            let held = (capacity != 0).then(|| ring.0.ring.lock().unwrap());
            tracing::subscriber::with_default(subscriber, || tracing::info!(sequence = 1));
            drop(held);
            assert!(ring.records().is_empty());
            assert_eq!(ring.dropped_records(), 1);
        }
    }

    #[test]
    fn disabled_subscriber_does_not_format_or_record_fields() {
        struct MustNotFormat;
        impl std::fmt::Debug for MustNotFormat {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("disabled event formatted its field");
            }
        }
        let ring = TraceRing::new(1, 512);
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_env_filter("off")
            .with_writer(ring.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || tracing::info!(field = ?MustNotFormat));
        assert!(ring.records().is_empty());
        assert_eq!(ring.dropped_records(), 0);
    }

    #[test]
    fn subscriber_output_retains_newest_complete_records_and_counts_loss() {
        let ring = TraceRing::new(2, 512);
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_writer(ring.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(sequence = 1);
            tracing::info!(sequence = 2);
            tracing::info!(sequence = 3);
        });
        let records = ring.records();
        let sequences: Vec<_> = records
            .iter()
            .map(|record| {
                let event: serde_json::Value = serde_json::from_slice(record).unwrap();
                event["fields"]["sequence"].as_u64().unwrap()
            })
            .collect();
        assert_eq!(sequences, [2, 3]);
        assert_eq!(ring.dropped_records(), 1);
    }

    #[test]
    fn oversized_subscriber_output_is_dropped_as_a_whole_record() {
        let ring = TraceRing::new(2, 128);
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_writer(ring.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(sequence = 1);
            tracing::info!(message = "x".repeat(512));
            tracing::info!(sequence = 2);
        });
        assert_eq!(ring.records().len(), 2);
        assert_eq!(ring.dropped_records(), 1);
        for record in ring.records() {
            assert!(serde_json::from_slice::<serde_json::Value>(&record).is_ok());
        }
    }
}
