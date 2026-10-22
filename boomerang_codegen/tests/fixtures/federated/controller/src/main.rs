#[path = "../../member.rs"]
mod member;

pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use boomerang_central_rti::compiled::hosted;
    use generated::{generated_bindings, wire_contract, FEDERATE, FEDERATE_VIEW};

    let Some(startup) = member::prepare()? else {
        return Ok(());
    };
    let connection = hosted::connect(startup.address, FEDERATE, wire_contract(), member::TIMEOUT)?;
    let sink = connection.sink();
    let bindings = generated_bindings(&startup.bindings, sink.clone())?;
    boomerang_runtime::execute_owned_federate_with_backend_and_observations(
        FEDERATE,
        &FEDERATE_VIEW,
        bindings,
        startup.config,
        &[],
        member::backend(sink, connection, startup.bindings),
    )?;
    println!("member completed");
    Ok(())
}
