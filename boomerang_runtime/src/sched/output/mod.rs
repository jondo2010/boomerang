//! Output completion owned by the scheduler.
//!
//! `state` retains requests and actual reaction outcomes. Tag checkpoints prove
//! completion through one logical tag. `closure` closes all microsteps through
//! a physical time, using the existing scheduler permissions and mailboxes.
//! Public outcome values and host observer handles remain in `crate::output`.

pub(super) mod closure;
mod state;

pub(crate) use closure::{
    progress::{IngressSeal, ProgressRelease},
    protocol::ProofFrame,
};
pub(crate) use state::OutputState;

use crate::sched::core::{ExecutionStorage, Schedule, SchedulerCore, SchedulerError};
use crate::{OutputBound, Tag};

impl<S: Schedule, E: ExecutionStorage<S>> SchedulerCore<'_, '_, S, E> {
    /// Called only after acquisition, upstream release, physical pacing and final input permission.
    /// The empty gap becomes real logical progress. This is never a ThroughTime projection.
    pub(in crate::sched) fn process_output_checkpoint(
        &mut self,
        tag: Tag,
    ) -> Result<bool, SchedulerError<E::Error>> {
        *self.current_tag = tag;
        self.release_tag_downstream(tag);
        if let Some(coordination) = self.federate_coordination.as_deref_mut() {
            if let Err(error) = coordination
                .logical_tag_complete(tag, false)
                .map_err(SchedulerError::FederateCoordination)
            {
                return Err(self.report_federate_failure(error));
            }
        }
        self.complete_output_bound(OutputBound::ThroughTag(tag));
        Ok(true)
    }
}
