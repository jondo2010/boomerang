//! Manifest support for the `cargo boomerang` deployment tool.

mod check;
mod driver;
mod generated;
mod manifest;
mod workspace;

pub use check::check;
pub use driver::{run_descriptor_driver, DriverOutput};
pub use manifest::{
    load_manifest, parse_manifest, Binding, Coordination, CoordinationBackend, Deployment,
    Federate, Manifest, Rti, Topology,
};
pub use workspace::{
    resolve_workspace, CargoPackage, LockfileIdentity, ResolvedFederate, ResolvedWorkspace,
};
