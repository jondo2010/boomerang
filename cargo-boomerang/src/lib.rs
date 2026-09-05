//! Manifest support for the `cargo boomerang` deployment tool.

mod build;
mod bundle;
mod check;
mod codegen;
mod driver;
mod generated;
mod generated_cache;
mod manifest;
mod output;
mod run;
mod workspace;

pub use build::{build, build_with_output};
pub use check::{check, check_with_output};
pub use codegen::{generate_launcher, BuiltLauncher, GeneratedLauncher};
pub use driver::{run_descriptor_driver, DriverOutput};
pub use manifest::{
    load_manifest, parse_manifest, Binding, Coordination, CoordinationBackend, Deployment,
    ExecutionPolicy, Federate, Manifest, Rti, Topology,
};
pub use output::{ColorChoice, CommandOutput};
pub use run::{run, run_with_output, ExecutionStats, ExecutionSummary, RunOutcome};
pub use workspace::{
    resolve_workspace, CargoPackage, LockfileIdentity, ResolvedFederate, ResolvedWorkspace,
};
