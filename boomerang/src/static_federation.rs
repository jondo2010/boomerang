//! Application-facing static federation execution.

use crate::{federated, runtime, BoomerangError};

/// Execute a lowered static federation using in-memory protocol transports.
pub fn execute_federation_in_memory(
    federation: federated::RuntimeFederation,
    config: runtime::Config,
) -> Result<federated::static_runner::FederationEnvs, BoomerangError> {
    federated::static_runner::run_in_memory(federation, config).map_err(BoomerangError::from)
}

/// Execute a lowered static federation using a runner-owned TCP listener.
pub fn execute_federation_over_tcp(
    federation: federated::RuntimeFederation,
    config: runtime::Config,
    tcp: federated::TcpStaticFederationConfig,
) -> Result<federated::static_runner::FederationEnvs, BoomerangError> {
    federated::static_runner::run_over_tcp(federation, config, tcp).map_err(BoomerangError::from)
}
