//! Deterministic source and manifest rendering for host descriptor drivers.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};

use crate::{CargoPackage, ResolvedWorkspace};

/// Complete in-memory contents of one generated descriptor-driver crate.
pub(crate) struct GeneratedCrate {
    /// Standalone Cargo manifest source.
    pub(crate) manifest: String,
    /// Generated Rust executable source.
    pub(crate) main: String,
}
/// Renders a standalone driver crate for one resolved deployment.
pub(crate) fn render_descriptor_driver(resolved: &ResolvedWorkspace) -> Result<GeneratedCrate> {
    let topology_package = resolved
        .package(&resolved.topology().package)
        .expect("resolved topology package is retained");
    let topology_crate = topology_package.lib_target.as_deref().ok_or_else(|| {
        anyhow!(
            "topology package '{}' does not expose a library target",
            topology_package.name
        )
    })?;
    let topology_entry = aliased_topology_entry(&resolved.topology().entry, topology_crate)?;

    let mut dependencies = BTreeMap::new();
    dependencies.insert(
        String::from("boomerang_builder"),
        dependency(
            resolved.host_builder(),
            false,
            vec![String::from("host-interchange")],
        )?,
    );
    dependencies.insert(
        String::from("topology_package"),
        dependency(topology_package, true, Vec::new())?,
    );

    let mut binding_expressions = Vec::new();
    for (index, (component, binding)) in resolved.deployment().bindings.iter().enumerate() {
        let alias = format!("implementation_{index}");
        let package = resolved
            .package(&binding.package)
            .expect("resolved implementation package is retained");
        let mut features = binding.features.clone();
        features.push(String::from("__boomerang_descriptor"));
        features.sort();
        features.dedup();
        dependencies.insert(alias.clone(), dependency(package, false, features)?);
        binding_expressions.push(format!(
            "DescriptorDriverBinding::new({component:?}, {package:?}, {alias}::__boomerang::descriptor())?",
            package = binding.package,
        ));
    }

    let package = toml::Table::from_iter([
        ("name".into(), "boomerang-descriptor-driver".into()),
        ("version".into(), "0.0.0".into()),
        ("edition".into(), "2021".into()),
        ("publish".into(), false.into()),
    ]);
    let manifest = toml::to_string(&toml::Table::from_iter([
        ("package".into(), package.into()),
        (
            "dependencies".into(),
            dependencies.into_iter().collect::<toml::Table>().into(),
        ),
        ("workspace".into(), toml::Table::new().into()),
    ]))
    .map_err(anyhow::Error::from)?;
    let bindings = binding_expressions.join(",\n        ");
    let main = format!(
        "use boomerang_builder::host_interchange::{{encode_descriptor_driver_output, \
         DescriptorDriverBinding, DescriptorDriverOutput}};\n\n\
         fn run() -> Result<(), Box<dyn std::error::Error>> {{\n\
         let topology_entry: fn() -> Result<boomerang_builder::compiler::ApplicationTopology, \
         boomerang_builder::compiler::TopologyBuildError> = {topology_entry};\n\
         let topology = topology_entry()?;\n\
         let output = DescriptorDriverOutput::try_new(topology, vec![{bindings}])?;\n\
         encode_descriptor_driver_output(std::io::stdout().lock(), output)?; Ok(()) }}\n\n\
         fn main() {{ if let Err(error) = run() {{ eprintln!(\"{{error}}\"); \
         std::process::exit(1); }} }}\n",
    );
    Ok(GeneratedCrate { manifest, main })
}
/// Converts one Cargo package identity into an exact generated dependency.
fn dependency(
    package: &CargoPackage,
    default_features: bool,
    features: Vec<String>,
) -> Result<toml::Value> {
    let mut rendered = toml::Table::from_iter([
        ("package".into(), package.name.clone().into()),
        ("default-features".into(), default_features.into()),
        (
            "features".into(),
            features
                .into_iter()
                .map(toml::Value::from)
                .collect::<Vec<_>>()
                .into(),
        ),
    ]);
    match package.source.as_deref() {
        None => {
            rendered.insert(
                "path".into(),
                package
                    .manifest_path
                    .parent()
                    .expect("package manifest has a parent")
                    .to_string_lossy()
                    .into_owned()
                    .into(),
            );
        }
        Some("registry+https://github.com/rust-lang/crates.io-index")
        | Some("registry+https://index.crates.io/") => {
            rendered.insert("version".into(), format!("={}", package.version).into());
        }
        Some(source) => {
            return Err(anyhow!(
                "unsupported Cargo source '{source}' for package '{}'",
                package.name
            ));
        }
    }
    Ok(rendered.into())
}
/// Rewrites an application entry path to the generated topology dependency alias.
fn aliased_topology_entry(entry: &str, expected_crate: &str) -> Result<String> {
    let mut segments = entry.split("::");
    let valid = segments.next() == Some(expected_crate)
        && segments.clone().next().is_some()
        && segments.clone().all(valid_rust_identifier);
    if !valid {
        return Err(anyhow!(
            "topology entry '{entry}' must be rooted at crate '{expected_crate}'"
        ));
    }
    Ok(std::iter::once("topology_package")
        .chain(segments)
        .collect::<Vec<_>>()
        .join("::"))
}
/// Accepts conservative Rust path segments used in generated source.
fn valid_rust_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
        && !matches!(value, "crate" | "self" | "super" | "Self")
}
