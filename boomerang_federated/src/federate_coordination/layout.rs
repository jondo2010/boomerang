#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FederateCoordinationLayout {
    participants: Vec<boomerang_runtime::EnclaveKey>,
}

impl FederateCoordinationLayout {
    pub(crate) fn new(
        participants: impl IntoIterator<Item = boomerang_runtime::EnclaveKey>,
    ) -> Self {
        let mut participants = participants.into_iter().collect::<Vec<_>>();
        participants.sort();
        participants.dedup();
        Self { participants }
    }

    pub(crate) fn participants(&self) -> &[boomerang_runtime::EnclaveKey] {
        &self.participants
    }
}
