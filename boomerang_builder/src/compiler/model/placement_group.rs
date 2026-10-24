use crate::compiler::PlacementGroupId;

/// Hierarchical placement-group declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementGroup {
    /// Stable placement-group identity.
    pub(super) id: PlacementGroupId,
    /// Optional parent placement-group identity.
    pub(super) parent: Option<PlacementGroupId>,
}

impl PlacementGroup {
    /// Returns the stable placement-group identity.
    pub fn id(&self) -> &PlacementGroupId {
        &self.id
    }

    /// Returns the optional parent placement-group identity.
    pub fn parent(&self) -> Option<&PlacementGroupId> {
        self.parent.as_ref()
    }
}
