//! Manifest support for the `cargo boomerang` deployment tool.

mod build;
mod bundle;
mod check;
mod codegen;
mod driver;
mod generated;
mod generated_cache;
mod manifest;
mod run;
mod workspace;

pub use build::build;
pub use check::check;
pub use codegen::{generate_launcher, BuiltLauncher, GeneratedLauncher};
pub use driver::{run_descriptor_driver, DriverOutput};
pub use manifest::{
    load_manifest, parse_manifest, Binding, Coordination, CoordinationBackend, Deployment,
    ExecutionPolicy, Federate, Manifest, Rti, Topology,
};
pub use run::{run, ExecutionStats, ExecutionSummary, RunOutcome};
pub use workspace::{
    resolve_workspace, CargoPackage, LockfileIdentity, ResolvedFederate, ResolvedWorkspace,
};
