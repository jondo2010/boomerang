//! Application-facing static federation execution.

use crate::{builder::RuntimeAssembly, central_rti, runtime, BoomerangError};

/// Execute a lowered static federation using in-memory protocol transports.
pub fn execute_federation_in_memory(
    parts: RuntimeAssembly,
    config: runtime::Config,
) -> Result<central_rti::static_runner::FederationEnvs, BoomerangError> {
    let federation = parts
        .federation
        .ok_or(BoomerangError::MissingStaticFederation)?;
    central_rti::static_runner::run_in_memory(federation.runtime, parts.enclaves, config)
        .map_err(BoomerangError::from)
}

/// Execute a lowered static federation using a runner-owned TCP listener.
pub fn execute_federation_over_tcp(
    parts: RuntimeAssembly,
    config: runtime::Config,
    tcp: central_rti::TcpStaticFederationConfig,
) -> Result<central_rti::static_runner::FederationEnvs, BoomerangError> {
    let federation = parts
        .federation
        .ok_or(BoomerangError::MissingStaticFederation)?;
    central_rti::static_runner::run_over_tcp(federation.runtime, parts.enclaves, config, tcp)
        .map_err(BoomerangError::from)
}
