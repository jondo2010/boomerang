pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use boomerang_central_rti::compiled::{hosted, CentralRtiClient, RtiClientBindings};
    use boomerang_runtime::image::RtiImageView;
    use generated::*;
    println!(
        "coordination={:?} mapping={:?}",
        COORDINATION_IDENTITY, WIRE_MAPPING
    );
    if std::env::args().any(|arg| arg == "--identity") {
        return Ok(());
    }
    let timeout = std::time::Duration::from_secs(10);
    let address = std::env::var("BOOMERANG_RTI_ADDRESS")?.parse()?;
    let view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS)?;
    let rti_bindings = RtiClientBindings::from_image(view, FEDERATE, COORDINATION_IDENTITY)?;
    let connection = hosted::connect(address, FEDERATE, wire_contract(), timeout)?;
    let sink = connection.sink();
    let bindings = generated_bindings(&rti_bindings, sink.clone())?;
    boomerang_runtime::execute_owned_federate_with_backend_and_observations(
        FEDERATE,
        &FEDERATE_VIEW,
        bindings,
        boomerang_runtime::Config::default().with_fast_forward(true),
        &[],
        |inbound| CentralRtiClient::connect(sink, connection, rti_bindings, inbound, timeout),
    )?;
    println!("member completed");
    Ok(())
}
