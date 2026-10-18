//! Reservations retained until all destinations in a batch complete.

use super::CompiledPhysicalAction;
use crate::Tag;

/// One outstanding destination/tag reservation, without payload ownership.
#[derive(Clone, Copy)]
pub(super) struct Reservation {
    pub(super) action: CompiledPhysicalAction,
    pub(super) tag: Tag,
}

/// Flat bounded reservation storage with exclusive ends for each outstanding batch.
/// A partially completed batch keeps its batch slot until its last reservation completes.
pub(super) struct RetainedBatches {
    pub(super) entries: Vec<Reservation>,
    ends: Vec<usize>,
}

impl RetainedBatches {
    /// Start empty. Storage capacity grows as batches are admitted.
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            ends: Vec::new(),
        }
    }

    /// Number of batches with at least one outstanding destination.
    pub(super) fn len(&self) -> usize {
        self.ends.len()
    }

    /// Store a validated nonempty batch. Reuse available storage capacity.
    pub(super) fn append(&mut self, reservations: &[Reservation]) {
        debug_assert!(!reservations.is_empty());
        self.entries.extend_from_slice(reservations);
        self.ends.push(self.entries.len());
    }

    /// Remove completed reservations and batches without allocating new storage.
    pub(super) fn reclaim(&mut self, mut pending: impl FnMut(&Reservation) -> bool) {
        let mut read_start = 0;
        let mut write = 0;
        let mut batches = 0;
        for batch in 0..self.ends.len() {
            let read_end = self.ends[batch];
            let write_start = write;
            for read in read_start..read_end {
                if pending(&self.entries[read]) {
                    self.entries[write] = self.entries[read];
                    write += 1;
                }
            }
            if write != write_start {
                self.ends[batches] = write;
                batches += 1;
            }
            read_start = read_end;
        }
        self.entries.truncate(write);
        self.ends.truncate(batches);
    }
}
