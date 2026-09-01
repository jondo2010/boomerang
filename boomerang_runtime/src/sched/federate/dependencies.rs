//! Logical-time dependency topology for one scheduler-owned Enclave.

use crate::{Duration, EnclaveKey, SendContext};

/// Crate-private logical-time relationships for the one Enclave owned by a compiled scheduler.
///
/// This is the scheduler-owned Enclave boundary: it describes only the
/// scheduler key and local logical-time links required to coordinate that
/// Enclave. Federate-wide quiescence remains outside this type.
pub(crate) struct EnclaveDependencies {
    /// Canonical identity of this scheduler-owned Enclave.
    pub(crate) key: EnclaveKey,
    /// Logical upstream Enclaves and minimum delays across parallel local routes.
    pub(crate) upstream: tinymap::TinySecondaryMap<EnclaveKey, (SendContext, Option<Duration>)>,
    /// Coalesced downstream Enclave contexts used for logical tag release.
    pub(crate) downstream: tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
}

impl EnclaveDependencies {
    /// Creates an unlinked dependency descriptor for one scheduler-owned Enclave.
    pub(crate) fn new(key: EnclaveKey) -> Self {
        Self {
            key,
            upstream: tinymap::TinySecondaryMap::new(),
            downstream: tinymap::TinySecondaryMap::new(),
        }
    }

    /// Adds one logical upstream, retaining the most restrictive delay for parallel routes.
    pub(crate) fn add_upstream(
        &mut self,
        key: EnclaveKey,
        context: SendContext,
        delay: Option<Duration>,
    ) {
        if let Some((_, existing_delay)) = self.upstream.get_mut(key) {
            *existing_delay = match (*existing_delay, delay) {
                (None, _) | (_, None) => None,
                (Some(existing), Some(candidate)) => Some(existing.min(candidate)),
            };
        } else {
            self.upstream.insert(key, (context, delay));
        }
    }

    /// Adds one logical downstream, coalescing parallel routes to the same Enclave.
    pub(crate) fn add_downstream(&mut self, key: EnclaveKey, context: SendContext) {
        if !self.downstream.contains_key(key) {
            self.downstream.insert(key, context);
        }
    }
}
