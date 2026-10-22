pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use boomerang_runtime::image::RtiImageView;
    use generated::*;
    use std::io::Write;
    println!(
        "coordination={:?} mapping={:?}",
        COORDINATION_IDENTITY, WIRE_MAPPING
    );
    if std::env::args().any(|arg| arg == "--identity") {
        return Ok(());
    }
    let timeout = std::time::Duration::from_secs(10);
    let view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS)?;
    let rti =
        boomerang_central_rti::compiled::CompiledRti::from_image(view, COORDINATION_IDENTITY)?;
    let bind = std::env::var("BOOMERANG_RTI_BIND").unwrap_or_else(|_| "127.0.0.1:0".into());
    let listener = std::net::TcpListener::bind(bind)?;
    if let Ok(ready) = std::env::var("BOOMERANG_RTI_READY_ADDRESS") {
        let mut ready = std::net::TcpStream::connect_timeout(&ready.parse()?, timeout)?;
        ready.set_write_timeout(Some(timeout))?;
        writeln!(ready, "BOOMERANG_RTI_READY_V1 {}", listener.local_addr()?)?;
    } else {
        println!("BOOMERANG_RTI_READY_V1 {}", listener.local_addr()?);
    }
    boomerang_central_rti::compiled::hosted::Server::new(listener, rti, wire_contract(), timeout)?
        .serve()?;
    println!("coordinator completed");
    Ok(())
}
