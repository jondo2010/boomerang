//! Target-side startup policy shared by the two ordinary Cargo members.
//! Each main owns its connection and calls the runtime executor explicitly.

use crate::generated::{
    COORDINATION_IDENTITY, COORDINATION_IMAGE, COORDINATION_MEMBERS, FEDERATE, WIRE_MAPPING,
};
use boomerang_central_rti::compiled::{
    hosted::HostedConnection, CentralRtiClient, RtiClientBindings, RtiRequestSink,
};
use boomerang_runtime::{
    image::{BoundaryId, RtiImageView},
    Config, FederateCoordinationError, InboundBoundaryAdapter,
};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};

/// Shared bound for hosted connection setup and backend admission.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Member facts prepared before its main acquires the hosted connection.
pub struct Startup {
    /// Coordinator address supplied through the existing runtime environment input.
    pub address: SocketAddr,
    /// Typed client bindings derived from this member's generated coordination image.
    pub bindings: RtiClientBindings<'static>,
    /// Fast-forward scheduler policy with coordinated natural completion.
    pub config: Config,
}

/// Print the artifact identity or prepare this member's unchanged execution policy.
pub fn prepare() -> Result<Option<Startup>, Box<dyn std::error::Error>> {
    println!(
        "coordination={:?} mapping={:?}",
        COORDINATION_IDENTITY, WIRE_MAPPING
    );
    if std::env::args().any(|arg| arg == "--identity") {
        return Ok(None);
    }
    let address = std::env::var("BOOMERANG_RTI_ADDRESS")?.parse()?;
    let view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS)?;
    let bindings = RtiClientBindings::from_image(view, FEDERATE, COORDINATION_IDENTITY)?;
    Ok(Some(Startup {
        address,
        bindings,
        config: Config::default().with_fast_forward(true),
    }))
}

/// Runtime-installed inbound adapters for the generated logical boundaries.
type InboundRoutes = BTreeMap<BoundaryId<'static>, InboundBoundaryAdapter>;

/// Transfer the main's connection into the backend after runtime route installation.
pub fn backend(
    sink: Arc<dyn RtiRequestSink>,
    connection: HostedConnection,
    bindings: RtiClientBindings<'static>,
) -> impl FnOnce(InboundRoutes) -> Result<CentralRtiClient, FederateCoordinationError> {
    move |inbound| CentralRtiClient::connect(sink, connection, bindings, inbound, TIMEOUT)
}
