//! Facet selection is added after Cargo resolves the user's own compiler flags.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};

/// Effective global compiler wrapper hidden when Boomerang installs its own
/// `RUSTC_WRAPPER`. Cargo continues to apply `RUSTC_WORKSPACE_WRAPPER` itself.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CompilerWrappers {
    rustc: Option<PathBuf>,
}

impl CompilerWrappers {
    /// Resolves Cargo's hierarchy and environment, then applies an optional
    /// file-valued `--config` wrapper with Cargo's CLI precedence.
    pub(crate) fn resolve(cwd: &Path, explicit: Option<&Path>) -> Result<Self> {
        // Cargo gives this special environment variable precedence even over
        // `--config`, and an explicitly empty value disables all wrappers.
        let special_rustc_wrapper = std::env::var_os("RUSTC_WRAPPER");
        let configured = cargo_config2::Config::load_with_cwd(cwd)
            .context("failed to resolve Cargo compiler-wrapper configuration")?;
        let mut rustc = configured.build.rustc_wrapper;
        if special_rustc_wrapper.is_none() {
            if let Some(path) = explicit {
                let config = cargo_config2::de::Config::load_file(path).with_context(|| {
                    format!(
                        "failed to resolve explicit Cargo configuration {}",
                        path.display()
                    )
                })?;
                if let Some(value) = config.build.rustc_wrapper {
                    rustc = Some(resolve_explicit_program(path, &value.val));
                }
            }
        }
        if let Some(value) = special_rustc_wrapper {
            rustc = (!value.is_empty()).then(|| PathBuf::from(value));
        }
        Ok(Self { rustc })
    }
}

fn resolve_explicit_program(config: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    let has_native_separator = value.contains('/') || cfg!(windows) && value.contains('\\');
    if path.is_absolute() || !has_native_separator {
        return path.to_owned();
    }
    config
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
        .join(value)
}

/// Names one isolated compiler context without adding Cargo features to user crates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Facet {
    Hosted,
    Descriptor,
    Payload,
}

impl Facet {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Hosted => "hosted",
            Self::Descriptor => "descriptor",
            Self::Payload => "payload",
        }
    }

    /// Chains an existing wrapper, leaving Cargo configuration and all rustflags untouched.
    pub(crate) fn configure(
        self,
        command: &mut Command,
        wrapper: &Path,
        configured: &CompilerWrappers,
    ) {
        command
            .env("RUSTC_WRAPPER", wrapper)
            .env("BOOMERANG_COMPILE_FACET", self.name());
        if let Some(previous) = configured.rustc.as_deref() {
            command.env("BOOMERANG_USER_RUSTC_WRAPPER", previous);
        } else {
            command.env_remove("BOOMERANG_USER_RUSTC_WRAPPER");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    const CHILD_EXPECTED_WRAPPER: &str = "BOOMERANG_TEST_EXPECTED_WRAPPER";
    const CHILD_WORKSPACE: &str = "BOOMERANG_TEST_WRAPPER_WORKSPACE";
    const CHILD_EXPLICIT_CONFIG: &str = "BOOMERANG_TEST_WRAPPER_EXPLICIT_CONFIG";

    #[test]
    fn wrapper_resolution_child() {
        let Some(expected) = std::env::var_os(CHILD_EXPECTED_WRAPPER) else {
            return;
        };
        let workspace = PathBuf::from(std::env::var_os(CHILD_WORKSPACE).unwrap());
        let explicit = PathBuf::from(std::env::var_os(CHILD_EXPLICIT_CONFIG).unwrap());
        let wrappers = CompilerWrappers::resolve(&workspace, Some(&explicit)).unwrap();
        let expected = (!expected.is_empty()).then(|| PathBuf::from(expected));
        assert_eq!(wrappers.rustc, expected);
    }

    #[test]
    fn special_wrapper_environment_precedes_explicit_config_in_isolated_processes() {
        let directory = tempfile::tempdir().unwrap();
        let explicit = directory.path().join("federate/.cargo/config.toml");
        std::fs::create_dir_all(explicit.parent().unwrap()).unwrap();
        std::fs::write(&explicit, "[build]\nrustc-wrapper = \"explicit-wrapper\"\n").unwrap();
        let run = |special: Option<&str>, cargo_build: Option<&str>, expected: &str| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "facet::tests::wrapper_resolution_child",
                    "--nocapture",
                ])
                .env(CHILD_EXPECTED_WRAPPER, expected)
                .env(CHILD_WORKSPACE, directory.path())
                .env(CHILD_EXPLICIT_CONFIG, &explicit)
                .env_remove("RUSTC_WRAPPER")
                .env_remove("CARGO_BUILD_RUSTC_WRAPPER");
            if let Some(value) = special {
                command.env("RUSTC_WRAPPER", value);
            }
            if let Some(value) = cargo_build {
                command.env("CARGO_BUILD_RUSTC_WRAPPER", value);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };

        run(Some("special-wrapper"), None, "special-wrapper");
        run(Some(""), None, "");
        run(None, Some("cargo-env-wrapper"), "explicit-wrapper");
    }

    #[test]
    fn explicit_relative_wrapper_uses_native_path_separator() {
        let config = Path::new("workspace/federate/.cargo/config.toml");
        let value = format!("tools{}wrapper", std::path::MAIN_SEPARATOR);
        assert_eq!(
            resolve_explicit_program(config, &value),
            Path::new("workspace/federate").join(value)
        );
        #[cfg(not(windows))]
        assert_eq!(
            resolve_explicit_program(config, r"tools\wrapper.exe"),
            Path::new(r"tools\wrapper.exe")
        );
    }

    #[test]
    fn resolves_hierarchical_and_explicit_wrappers_without_overriding_workspace_wrapper() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join(".cargo")).unwrap();
        std::fs::write(
            directory.path().join(".cargo/config.toml"),
            "[build]\nrustc-wrapper = \"configured-global\"\nrustc-workspace-wrapper = \"configured-workspace\"\n",
        )
        .unwrap();
        let explicit = directory.path().join("federate/.cargo/config.toml");
        std::fs::create_dir_all(explicit.parent().unwrap()).unwrap();
        std::fs::write(
            &explicit,
            "[build]\nrustc-wrapper = \"bin/federate-wrapper\"\n",
        )
        .unwrap();

        let wrappers = CompilerWrappers::resolve(directory.path(), Some(&explicit)).unwrap();
        assert_eq!(
            wrappers.rustc,
            Some(directory.path().join("federate/bin/federate-wrapper"))
        );
        let mut command = Command::new("cargo");
        Facet::Payload.configure(&mut command, Path::new("boomerang-wrapper"), &wrappers);
        let user_wrapper = command
            .get_envs()
            .find(|(key, _)| *key == "BOOMERANG_USER_RUSTC_WRAPPER")
            .and_then(|(_, value)| value);
        assert_eq!(
            user_wrapper.map(Path::new),
            Some(
                directory
                    .path()
                    .join("federate/bin/federate-wrapper")
                    .as_path()
            )
        );
        assert!(!command
            .get_envs()
            .any(|(key, _)| key == "RUSTC_WORKSPACE_WRAPPER"));
    }

    #[test]
    #[cfg(unix)]
    fn compiler_wrapper_scopes_flags_chains_user_wrapper_and_rejects_conflicts() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("wrapper.rs");
        let executable = directory.path().join("wrapper");
        std::fs::write(&source, include_str!("facet_rustc.rs")).unwrap();
        assert!(Command::new("rustc")
            .arg("--edition=2021")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap()
            .success());
        let compiler = directory.path().join("compiler");
        std::fs::write(&compiler, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
        let prior = directory.path().join("prior");
        std::fs::write(&prior, "#!/bin/sh\necho user-wrapper\nexec \"$@\"\n").unwrap();
        std::fs::set_permissions(&prior, std::fs::Permissions::from_mode(0o755)).unwrap();
        let invoke = |arguments: &[&str]| {
            Command::new(&executable)
                .arg(&compiler)
                .args(arguments)
                .env("BOOMERANG_COMPILE_FACET", "descriptor")
                .env("BOOMERANG_USER_RUSTC_WRAPPER", &prior)
                .output()
                .unwrap()
        };
        let host = invoke(&["--crate-type", "proc-macro"]);
        assert!(host.status.success());
        assert_eq!(
            String::from_utf8(host.stdout).unwrap(),
            "user-wrapper\n--crate-type\nproc-macro\n"
        );
        let target = invoke(&["--target=some-target", "--cfg", "boomerang_facet_extra"]);
        assert!(target.status.success());
        let flags = String::from_utf8(target.stdout).unwrap();
        assert!(flags.contains("boomerang_facet_extra\n"));
        assert!(flags.contains("--cfg\nboomerang_facet=\"descriptor\"\n"));
        let conflict = invoke(&[
            "--target",
            "some-target",
            "--cfg=boomerang_facet=\"payload\"",
        ]);
        assert!(!conflict.status.success());
        assert!(String::from_utf8(conflict.stderr)
            .unwrap()
            .contains("conflict"));
    }
}
