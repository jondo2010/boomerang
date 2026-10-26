fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo = boomerang_codegen::cargo_build::CargoBuild::from_env()?;
    cargo.track_file("build.rs")?;
    for name in native_vehicle_deployment::CONFIGURATION_ENV {
        cargo.track_env(name)?;
    }
    let settings = native_vehicle_deployment::Settings::from_env()?;
    let compilation =
        native_vehicle_deployment::compilation(&settings, "host", cargo.target().clone())?;
    cargo.compile(compilation, &mut std::io::stdout())?;
    Ok(())
}
