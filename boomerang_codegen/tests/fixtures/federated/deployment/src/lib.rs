//! Shared host-only deployment for independently built vehicle applications.
//!
//! Cargo build dependencies use this crate for topology, descriptors, and configuration.
//! Target dependencies select only their owning component's `payload` feature.
//! The coordinator has no component target dependencies.
//! Each checked-in main owns its listener or connection and finite execution.
//! Generated definitions use the same canonical coordination and wire mapping identities.
//! The local scheduler image remains a separate identity domain.
//!
//! The native fixture explicitly declares both member targets as the build host triple.
//! `NATIVE_VEHICLE_HOST_TARGET` and `NATIVE_VEHICLE_SENSOR_TARGET` override those declarations.
//! `NATIVE_VEHICLE_DELAY_NS` overrides the default 1 ms boundary delay.
//! Every package tracks these inputs. All artifacts in one deployment require identical values.
//! Compiler validation checks each selected member against Cargo's actual `TARGET`.
//! Pure generation accepts heterogeneous declarations without cross-target execution claims.
//!
//! Build each package from this fixture workspace:
//! ```text
//! cargo build --locked -p native-vehicle-controller
//! cargo build --locked -p native-vehicle-sensor
//! cargo build --locked -p native-vehicle-rti
//! cargo test --locked -p native-vehicle-deployment
//! ```
//! Run `bash boomerang_codegen/tests/run-applications.sh` from the repository root.
//! The script builds immutable executable snapshots before the process tests start.
//! The process tests use ephemeral loopback ports and the RTI readiness protocol.
//! The actual wire handshake rejects artifacts from different deployments.
//! The same process harness runs the sensor's `runtime-failure` feature after successful admission.
//! It checks exit code 42 and bounds failed peer and coordinator teardown.
//! The checked-in RTI package tests its generated in-transit capacity through actual request handling.
//! Generic wire codec and admission cases remain in `boomerang_federated` tests.

use boomerang_builder::compiler::*;
use boomerang_codegen::{
    Compilation, DeploymentConfiguration, DeploymentInputs, PayloadExports, TargetContext,
};
use boomerang_runtime::{
    image::{
        BoundaryFailurePolicy, CodecPolicy, RecoveryPolicy, SecurityPolicy, TimingPolicy,
        TransportPolicy,
    },
    Duration,
};

/// Environment inputs tracked by every application build script.
pub const CONFIGURATION_ENV: [&str; 3] = [
    "NATIVE_VEHICLE_HOST_TARGET",
    "NATIVE_VEHICLE_SENSOR_TARGET",
    "NATIVE_VEHICLE_DELAY_NS",
];

/// Explicit member targets and logical route delay shared by every artifact.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Declared target for the controller and idle backup member.
    pub host_target: String,
    /// Declared target for the sensor member.
    pub sensor_target: String,
    /// Delay from controller command emission to sensor admission, in nanoseconds.
    pub delay_ns: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            host_target: env!("NATIVE_VEHICLE_BUILD_HOST").into(),
            sensor_target: env!("NATIVE_VEHICLE_BUILD_HOST").into(),
            delay_ns: 1_000_000,
        }
    }
}
impl Settings {
    /// Read the explicit fixture policy and optional deployment overrides.
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let defaults = Self::default();
        Ok(Self {
            host_target: std::env::var(CONFIGURATION_ENV[0]).unwrap_or(defaults.host_target),
            sensor_target: std::env::var(CONFIGURATION_ENV[1]).unwrap_or(defaults.sensor_target),
            delay_ns: std::env::var(CONFIGURATION_ENV[2])
                .map_or(Ok(defaults.delay_ns), |value| value.parse())?,
        })
    }
}

/// Build the existing vehicle topology from its owning component declarations.
pub fn inputs(settings: &Settings) -> Result<DeploymentInputs, Box<dyn std::error::Error>> {
    let mut app = TopologyBuilder::new("application/backup/controller/sensor")?;
    let controller_enclave = app.enclave("controller")?;
    let backup_enclave = app.enclave("backup")?;
    let sensor_enclave = app.enclave("sensor")?;
    let controller = app.component(
        "controller",
        vehicle_control::controller::definition(),
        &controller_enclave,
    )?;
    app.component(
        "backup",
        vehicle_control::controller::definition(),
        &backup_enclave,
    )?;
    let sensor = app.component("sensor", sensor_host::sensor::definition(), &sensor_enclave)?;
    app.connect_with_semantics(
        &controller.command,
        &sensor.command,
        ConnectionSemantics::Logical {
            after: Some(Duration::nanoseconds_i128(settings.delay_ns.into())),
        },
    )?;
    let host = FederateId::new("host")?;
    let sensor = FederateId::new("sensor")?;
    Ok(DeploymentInputs {
        catalog: [
            (
                ImplementationId::new("vehicle-control")?,
                vehicle_control::controller::__boomerang::metadata::descriptor(),
            ),
            (
                ImplementationId::new("sensor-host")?,
                sensor_host::sensor::__boomerang::metadata::descriptor(),
            ),
        ]
        .into_iter()
        .collect::<ImplementationCatalog>()
        .into(),
        topology: app.finish()?,
        configuration: DeploymentConfiguration {
            name: "native-vehicle".into(),
            placements: vec![
                PlacementAssignment::new(
                    PlacementGroupId::new("placement/controller")?,
                    host.clone(),
                ),
                PlacementAssignment::new(PlacementGroupId::new("placement/backup")?, host.clone()),
                PlacementAssignment::new(
                    PlacementGroupId::new("placement/sensor")?,
                    sensor.clone(),
                ),
            ],
            federates: vec![
                FederateConfig::new(
                    host,
                    TargetTriple::new(settings.host_target.as_str())?,
                    RuntimeBackendId::new("std")?,
                    RecoveryPolicy::FailStop,
                ),
                FederateConfig::new(
                    sensor,
                    TargetTriple::new(settings.sensor_target.as_str())?,
                    RuntimeBackendId::new("std")?,
                    RecoveryPolicy::FailStop,
                ),
            ],
            coordination: CoordinationSelection::Distributed {
                backend: CoordinationBackend::CentralRti,
            },
            boundaries: vec![BoundaryBinding::new(
                BoundaryId::new("boundary/controller%2Fcommand/sensor%2Fcommand/c0")?,
                FlowId::new("sensor-control")?,
                PhysicalBoundaryMetadata::new(None, None),
                CodecCapabilityId::new("serde-json")?,
                TransportCapabilityId::new("tcp")?,
                BoundaryPolicies::new(
                    BoundaryFailurePolicy::PropagateStop,
                    TransportPolicy::ReliableOrderedFramed,
                    CodecPolicy::CanonicalBounded,
                    TimingPolicy::BestEffort,
                    SecurityPolicy::None,
                ),
            )],
        },
        bindings: vec![
            ImplementationBinding::new(
                ComponentInstanceId::new("controller")?,
                ImplementationId::new("vehicle-control")?,
            ),
            ImplementationBinding::new(
                ComponentInstanceId::new("backup")?,
                ImplementationId::new("vehicle-control")?,
            ),
            ImplementationBinding::new(
                ComponentInstanceId::new("sensor")?,
                ImplementationId::new("sensor-host")?,
            ),
        ],
    })
}

/// Select one member while retaining all shared topology and semantic validation inputs.
pub fn compilation(
    settings: &Settings,
    member: &str,
    target: TargetContext,
) -> Result<Compilation, Box<dyn std::error::Error>> {
    let inputs = inputs(settings)?;
    let mut result = Compilation::new(
        inputs.topology,
        inputs.configuration,
        FederateId::new(member)?,
        target,
    );
    result.register(
        ImplementationId::new("vehicle-control")?,
        vehicle_control::controller::__boomerang::metadata::descriptor(),
        PayloadExports::new("vehicle_control::controller::__boomerang::payload")?,
    )?;
    result.register(
        ImplementationId::new("sensor-host")?,
        sensor_host::sensor::__boomerang::metadata::descriptor(),
        PayloadExports::new("sensor_host::sensor::__boomerang::payload")?,
    )?;
    for binding in inputs.bindings {
        result.select(
            binding.component().clone(),
            binding.implementation().clone(),
        )?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heterogeneous_generation_preserves_targets_and_local_payloads() {
        let settings = Settings {
            host_target: "x86_64-unknown-linux-gnu".into(),
            sensor_target: "aarch64-unknown-linux-gnu".into(),
            delay_ns: 1_000_000,
        };
        let coordinator = inputs(&settings).unwrap().compile_coordination().unwrap();
        assert!(!coordinator.source.contains("vehicle_control"));
        assert!(!coordinator.source.contains("sensor_host"));
        for (member, target, local, remote) in [
            (
                "host",
                &settings.host_target,
                "vehicle_control::",
                "sensor_host::",
            ),
            (
                "sensor",
                &settings.sensor_target,
                "sensor_host::",
                "vehicle_control::",
            ),
        ] {
            let generated = compilation(&settings, member, TargetContext::new(target).unwrap())
                .unwrap()
                .compile()
                .unwrap();
            assert!(generated.source.contains(local));
            assert!(!generated.source.contains(remote));
            assert_eq!(
                boomerang_codegen::coordination::coordination_fingerprint(
                    &generated.image,
                )
                .unwrap(),
                boomerang_codegen::coordination::coordination_fingerprint(
                    &coordinator.image,
                )
                .unwrap()
            );
            let error = compilation(
                &settings,
                member,
                TargetContext::new("wasm32-unknown-unknown").unwrap(),
            )
            .unwrap()
            .compile()
            .unwrap_err();
            assert_eq!(
                error.diagnostic.code,
                boomerang_codegen::DiagnosticCode::TargetMismatch
            );
        }
    }
}
