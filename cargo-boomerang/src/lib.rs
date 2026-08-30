//! Manifest support for the `cargo boomerang` deployment tool.

mod driver;
mod generated;
mod manifest;
mod workspace;

pub use driver::{run_descriptor_driver, DescriptorDriverError, DriverOutput};
pub use manifest::{
    load_manifest, parse_manifest, Binding, Coordination, CoordinationBackend, Deployment,
    Federate, Manifest, ManifestError, Rti, Topology,
};
pub use workspace::{
    resolve_workspace, CargoPackage, LockfileIdentity, ResolvedFederate, ResolvedWorkspace,
    WorkspaceError,
};
