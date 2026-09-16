//! Deterministic source and manifest rendering for host descriptor drivers.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use quote::quote;

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
    render_host_driver(resolved, false)
}

/// Generates the ordinary authoring stage without any descriptor dependencies.
pub(crate) fn render_topology_driver(resolved: &ResolvedWorkspace) -> Result<GeneratedCrate> {
    render_host_driver(resolved, true)
}

fn render_host_driver(resolved: &ResolvedWorkspace, topology: bool) -> Result<GeneratedCrate> {
    let mut dependencies = BTreeMap::new();
    dependencies.insert(
        "boomerang_builder".to_owned(),
        dependency(
            resolved.host_builder(),
            false,
            vec!["host-interchange".to_owned()],
        )?,
    );
    let body = if topology {
        let package = resolved
            .package(&resolved.topology().package)
            .expect("topology resolved");
        let entry = aliased_topology_entry(
            &resolved.topology().entry,
            package
                .lib_target
                .as_deref()
                .ok_or_else(|| anyhow!("topology package has no library"))?,
        )?;
        dependencies.insert(
            "topology_package".to_owned(),
            dependency(package, true, Vec::new())?,
        );
        let entry: syn::Path = syn::parse_str(&entry)?;
        quote! {
            let topology = #entry()?;
            boomerang_builder::host_interchange::encode_topology_output(std::io::stdout().lock(), topology)?;
        }
    } else {
        let mut selected = BTreeMap::<String, Vec<String>>::new();
        for binding in resolved.deployment().bindings.values() {
            selected
                .entry(binding.package.clone())
                .or_default()
                .extend(binding.features.iter().cloned());
        }
        let mut aliases = BTreeMap::new();
        for (index, (name, mut features)) in selected.into_iter().enumerate() {
            let package = resolved.package(&name).expect("implementation resolved");
            if package.legacy_facets {
                features.push("__boomerang_descriptor".to_owned());
            }
            features.sort();
            features.dedup();
            let alias = format!("implementation_{index}");
            dependencies.insert(alias.clone(), dependency(package, false, features)?);
            aliases.insert(name, alias);
        }
        let bindings = resolved
            .deployment()
            .bindings
            .iter()
            .map(|(component, binding)| {
                let path: syn::Path =
                    syn::parse_str(&binding.exported_path(&aliases[&binding.package]))?;
                let implementation = binding.implementation_id();
                Ok(quote! {
                    boomerang_builder::host_interchange::DescriptorDriverBinding::new(
                        #component, #implementation, #path::__boomerang::descriptor(),
                    )?
                })
            })
            .collect::<Result<Vec<_>>>()?;
        quote! {
            boomerang_builder::host_interchange::encode_descriptor_output(
                std::io::stdout().lock(), vec![#(#bindings),*],
            )?;
        }
    };
    let name = if topology {
        "boomerang-topology-driver"
    } else {
        "boomerang-descriptor-driver"
    };
    let package = toml::Table::from_iter([
        ("name".into(), name.into()),
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
    ]))?;
    let main = crate::codegen::format_rust(quote! {
        fn run() -> Result<(), Box<dyn std::error::Error>> {
            #body
            Ok(())
        }
        fn main() {
            if let Err(error) = run() {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    })?;
    Ok(GeneratedCrate { manifest, main })
}
/// Converts one Cargo package identity into an exact generated dependency.
pub(crate) fn dependency(
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

/// Renders a same-release Boomerang package beside the resolved runtime package.
pub(crate) fn runtime_sibling_dependency(
    runtime: &CargoPackage,
    package: &str,
    features: Vec<String>,
) -> Result<toml::Value> {
    let mut rendered = dependency(runtime, false, features)?;
    let table = rendered
        .as_table_mut()
        .expect("dependency renders as a TOML table");
    table.insert("package".into(), package.into());
    if runtime.source.is_none() {
        let workspace = runtime
            .manifest_path
            .parent()
            .and_then(std::path::Path::parent)
            .expect("local runtime package has a workspace parent");
        table.insert(
            "path".into(),
            workspace
                .join(package)
                .to_string_lossy()
                .into_owned()
                .into(),
        );
    }
    Ok(rendered)
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
