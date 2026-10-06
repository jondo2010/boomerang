//! `RUSTC_WRAPPER` protocol for facet-specific compiler flags.

use std::{env, ffi::OsString, process::Command};

use anyhow::{bail, Context, Result};

/// Runs the configured compiler command and exits with its status.
pub fn main() -> ! {
    let result = (|| -> Result<i32> {
        let mut arguments = env::args_os().skip(1);
        let compiler = arguments.next().context("missing compiler command")?;
        let mut arguments: Vec<OsString> = arguments.collect();
        let mode =
            env::var("BOOMERANG_COMPILE_FACET").context("missing BOOMERANG_COMPILE_FACET")?;
        if !matches!(mode.as_str(), "hosted" | "descriptor" | "payload") {
            bail!("invalid tool-owned compilation facet");
        }
        // Cargo passes an explicit target for target crates and target cfg discovery,
        // but not for build scripts, proc macros, or their host dependencies.
        if arguments
            .iter()
            .any(|arg| arg == "--target" || arg.to_string_lossy().starts_with("--target="))
        {
            for (index, argument) in arguments.iter().enumerate() {
                let value = if argument == "--cfg" {
                    arguments
                        .get(index + 1)
                        .map(|value| value.to_string_lossy())
                } else {
                    argument
                        .to_str()
                        .and_then(|value| value.strip_prefix("--cfg="))
                        .map(Into::into)
                };
                if let Some(value) =
                    value.filter(|value| value.split('=').next() == Some("boomerang_facet"))
                {
                    if value != format!("boomerang_facet=\"{mode}\"") {
                        bail!("compiler flags conflict with the {mode} facet: {value}");
                    }
                }
            }
            arguments
                .push("--check-cfg=cfg(boomerang_facet,values(\"descriptor\",\"payload\"))".into());
            if mode != "hosted" {
                arguments.extend(["--cfg".into(), format!("boomerang_facet=\"{mode}\"").into()]);
            }
        }
        let previous =
            env::var_os("BOOMERANG_USER_RUSTC_WRAPPER").filter(|value| !value.is_empty());
        let mut command = if let Some(previous) = previous {
            let mut command = Command::new(previous);
            command.arg(compiler);
            command
        } else {
            Command::new(compiler)
        };
        let status = command
            .args(arguments)
            .status()
            .context("failed to run compiler command")?;
        Ok(status.code().unwrap_or(1))
    })();
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("boomerang compiler configuration: {error:#}");
            1
        }
    };
    std::process::exit(code)
}
