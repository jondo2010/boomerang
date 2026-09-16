//! Source of the small, generated host compiler wrapper. Uses only the standard library.

use std::{env, ffi::OsString, process::{Command, ExitCode}};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => { eprintln!("boomerang compiler configuration: {error}"); ExitCode::FAILURE }
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let mut arguments = env::args_os().skip(1);
    let compiler = arguments.next().ok_or("missing compiler command")?;
    let mut arguments: Vec<OsString> = arguments.collect();
    let mode = env::var("BOOMERANG_COMPILE_FACET")?;
    if !matches!(mode.as_str(), "hosted" | "descriptor" | "payload") {
        return Err("invalid tool-owned compilation facet".into());
    }
    // Cargo passes an explicit target for target crates and target cfg discovery,
    // but not for build scripts, proc macros, or their host dependencies.
    if arguments.iter().any(|arg| arg == "--target" || arg.to_string_lossy().starts_with("--target=")) {
        for (index, argument) in arguments.iter().enumerate() {
            let value = if argument == "--cfg" {
                arguments.get(index + 1).map(|value| value.to_string_lossy())
            } else {
                argument.to_str().and_then(|value| value.strip_prefix("--cfg=")).map(Into::into)
            };
            if let Some(value) = value.filter(|value| value.split('=').next() == Some("boomerang_facet")) {
                if value != format!("boomerang_facet=\"{mode}\"") {
                    return Err(format!("compiler flags conflict with the {mode} facet: {value}").into());
                }
            }
        }
        arguments.push("--check-cfg=cfg(boomerang_facet,values(\"descriptor\",\"payload\"))".into());
        if mode != "hosted" {
            arguments.extend(["--cfg".into(), format!("boomerang_facet=\"{mode}\"").into()]);
        }
    }
    let previous = env::var_os("BOOMERANG_USER_RUSTC_WRAPPER").filter(|value| !value.is_empty());
    let mut command = if let Some(previous) = previous {
        let mut command = Command::new(previous);
        command.arg(compiler);
        command
    } else {
        Command::new(compiler)
    };
    let status = command.args(arguments).status()?;
    Ok(status.code().and_then(|code| u8::try_from(code).ok()).unwrap_or(1))
}
