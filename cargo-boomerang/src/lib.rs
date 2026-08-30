//! Manifest support for the `cargo boomerang` deployment tool.

mod manifest;
mod workspace;

pub use manifest::{
    load_manifest, parse_manifest, Binding, Coordination, CoordinationBackend, Deployment,
    Federate, Manifest, ManifestError, Rti, Topology,
};
pub use workspace::{
    resolve_workspace, FacetFeatures, LockfileIdentity, ResolvedFederate, ResolvedPackage,
    ResolvedTopology, ResolvedWorkspace, WorkspaceError,
};
