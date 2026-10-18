//! Source declarations and validation of compiled action references.

use super::{CompiledPhysicalAction, InputError, SourceKey};
use crate::{
    image::{ActionTiming, EnclaveImageView, EnclaveIndex, TimingDomain},
    Duration,
};

/// The policy and physical actions for one input source.
pub struct InputSource {
    /// The source name used in diagnostics. It does not determine routing.
    pub id: String,
    /// Required sources limit execution. A protocol error, overflow, or disconnection aborts the Federate.
    pub required: bool,
    /// The physical actions owned by this source.
    pub actions: Vec<CompiledPhysicalAction>,
}
/// Source declarations and batch capacity limits.
pub struct InputConfig {
    /// The maximum number of values in one batch.
    pub max_batch_values: usize,
    /// The maximum number of batches retained until their destinations complete.
    pub max_staged_batches: usize,
    /// The complete source table supplied by the compiler.
    pub sources: tinymap::TinyMap<SourceKey, InputSource>,
}

impl InputConfig {
    /// Validate compiled references before initializers or drivers run.
    pub(crate) fn validate(
        &self,
        images: &tinymap::TinySecondaryMap<EnclaveIndex, EnclaveImageView<'_>>,
    ) -> Result<tinymap::TinyMap<SourceKey, Duration>, InputError> {
        if self.max_batch_values == 0
            || self.max_staged_batches == 0
            || self
                .max_batch_values
                .checked_mul(self.max_staged_batches)
                .is_none()
        {
            return Err(InputError::configuration_error());
        }
        let mut owned = Vec::new();
        let mut ids = std::collections::BTreeSet::new();
        self.sources
            .values()
            .map(|source| {
                if source.id.is_empty() || !ids.insert(&source.id) || source.actions.is_empty() {
                    return Err(InputError::configuration_error());
                }
                let mut delay = None;
                for reference in &source.actions {
                    if owned.contains(reference) {
                        return Err(InputError::configuration_error());
                    }
                    owned.push(*reference);
                    let image = images
                        .get(reference.enclave)
                        .ok_or_else(InputError::configuration_error)?;
                    let action = image
                        .actions()
                        .get(reference.action)
                        .ok_or_else(InputError::configuration_error)?;
                    let ActionTiming::Standard {
                        domain: TimingDomain::Physical,
                        min_delay_nanos,
                    } = action.timing()
                    else {
                        return Err(InputError::configuration_error());
                    };
                    if action.binding().is_none() {
                        return Err(InputError::configuration_error());
                    }
                    let value = Duration::nanoseconds(
                        i64::try_from(*min_delay_nanos)
                            .map_err(|_| InputError::configuration_error())?,
                    );
                    delay = Some(delay.map_or(value, |old: Duration| old.min(value)));
                }
                delay.ok_or_else(InputError::configuration_error)
            })
            .collect()
    }
}
