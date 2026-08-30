//! Manifest support for the `cargo boomerang` deployment tool.

mod manifest;

pub use manifest::{
    load_manifest, parse_manifest, Binding, Coordination, CoordinationBackend, Deployment,
    Federate, Manifest, ManifestError, Rti, Topology,
};
