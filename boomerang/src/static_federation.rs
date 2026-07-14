//! Application-facing static federation execution.

use crate::{builder::RuntimeAssembly, federated, runtime, BoomerangError};

/// Execute a lowered static federation using in-memory protocol transports.
pub fn execute_federation_in_memory(
    parts: RuntimeAssembly,
    config: runtime::Config,
) -> Result<federated::static_runner::FederationEnvs, BoomerangError> {
    let federation = parts
        .federation
        .ok_or(BoomerangError::MissingStaticFederation)?;
    federated::static_runner::run_in_memory(federation.runtime, parts.enclaves, config)
        .map_err(BoomerangError::from)
}

/// Execute a lowered static federation using a runner-owned TCP listener.
pub fn execute_federation_over_tcp(
    parts: RuntimeAssembly,
    config: runtime::Config,
    tcp: federated::TcpStaticFederationConfig,
) -> Result<federated::static_runner::FederationEnvs, BoomerangError> {
    let federation = parts
        .federation
        .ok_or(BoomerangError::MissingStaticFederation)?;
    federated::static_runner::run_over_tcp(federation.runtime, parts.enclaves, config, tcp)
        .map_err(BoomerangError::from)
}
