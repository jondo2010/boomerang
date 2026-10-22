//! Optional Cargo build-script integration. No nested Cargo invocation or discovery.
//!
//! Construct with [`CargoBuild::from_env`], declare all files and environment variables
//! used to construct your topology, and call [`CargoBuild::compile`] with stdout.
//! The fixed output filename is `boomerang.rs`; applications include it under a module.
use crate::{Compilation, CompilationError, DiagnosticCode, Stage, TargetContext};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Owns only `OUT_DIR/boomerang.rs` and temporary files created for its publication.
#[derive(Debug)]
pub struct CargoBuild {
    output: PathBuf,
    target: TargetContext,
    files: BTreeSet<String>,
    environment: BTreeSet<String>,
}
impl CargoBuild {
    /// Capture Cargo's target facts and output directory. Missing TARGET is an error.
    pub fn from_env() -> Result<Self, CompilationError> {
        let values = std::env::vars_os()
            .filter_map(|(key, value)| {
                let key = key.into_string().ok()?;
                (key == "TARGET" || key == "OUT_DIR" || key.starts_with("CARGO_CFG_"))
                    .then_some((key, value))
            })
            .map(|(key, value)| {
                value
                    .into_string()
                    .map(|value| (key, value))
                    .map_err(|_| invalid("Cargo context is not UTF-8"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Self::from_values(&values)
    }
    /// Inject Cargo values without mutating process-global environment (useful in tests).
    pub fn from_values(values: &BTreeMap<String, String>) -> Result<Self, CompilationError> {
        let triple = values.get("TARGET").ok_or_else(|| {
            CompilationError::new(
                Stage::Cargo,
                DiagnosticCode::MissingTarget,
                "Cargo TARGET is required",
            )
        })?;
        directive_value(triple)?;
        let output = values
            .get("OUT_DIR")
            .ok_or_else(|| invalid("Cargo OUT_DIR is required"))?;
        let mut cfg = BTreeMap::new();
        for (key, value) in values
            .iter()
            .filter(|(key, _)| key.starts_with("CARGO_CFG_"))
        {
            env_name(key)?;
            directive_value(value)?;
            cfg.insert(key.clone(), value.clone());
        }
        Self::from_context(output, TargetContext::new(triple)?.with_cfg(cfg))
    }
    /// Use an explicit target and output directory, without environment access.
    /// Only the fixed `boomerang.rs` child is published; no output filename is accepted.
    pub fn from_context(
        output: impl AsRef<Path>,
        target: TargetContext,
    ) -> Result<Self, CompilationError> {
        if output.as_ref().as_os_str().is_empty() {
            return Err(invalid("output directory is empty"));
        }
        let mut environment = BTreeSet::from(["TARGET".into()]);
        environment.extend(target.cfg().keys().cloned());
        // Track well-known facts even when absent in an injected context.
        environment.extend(
            [
                "CARGO_CFG_TARGET_ARCH",
                "CARGO_CFG_TARGET_OS",
                "CARGO_CFG_TARGET_ENV",
                "CARGO_CFG_TARGET_VENDOR",
                "CARGO_CFG_TARGET_ENDIAN",
                "CARGO_CFG_TARGET_POINTER_WIDTH",
                "CARGO_CFG_TARGET_FEATURE",
                "CARGO_CFG_TARGET_FAMILY",
                "CARGO_CFG_TARGET_HAS_ATOMIC",
                "CARGO_CFG_PANIC",
            ]
            .map(str::to_owned),
        );
        Ok(Self {
            output: output.as_ref().to_owned(),
            target,
            files: BTreeSet::new(),
            environment,
        })
    }
    pub fn target(&self) -> &TargetContext {
        &self.target
    }
    /// Track a source file or directory read by the build script, relative to its package.
    pub fn track_file(&mut self, path: impl AsRef<Path>) -> Result<(), CompilationError> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or_else(|| invalid("tracked path is not UTF-8"))?;
        directive_value(path)?;
        if path.is_empty() {
            return Err(invalid("tracked path is empty"));
        }
        self.files.insert(path.into());
        Ok(())
    }
    /// Track an additional environment input; names must be portable identifiers.
    pub fn track_env(&mut self, name: impl AsRef<str>) -> Result<(), CompilationError> {
        let name = name.as_ref();
        env_name(name)?;
        self.environment.insert(name.into());
        Ok(())
    }
    /// Compile, then atomically publish complete source. Identical bytes preserve mtime.
    /// Any failure invalidates prior success output; unrelated OUT_DIR files survive.
    /// Pass `&mut std::io::stdout()` from build.rs to emit Cargo rerun directives.
    pub fn compile(
        &self,
        compilation: &Compilation,
        directives: &mut impl Write,
    ) -> Result<PathBuf, CompilationError> {
        let destination = self.output.join("boomerang.rs");
        let result = (|| {
            for file in &self.files {
                writeln!(directives, "cargo:rerun-if-changed={file}").map_err(io_error)?;
            }
            for name in &self.environment {
                writeln!(directives, "cargo:rerun-if-env-changed={name}").map_err(io_error)?;
            }
            if compilation.target() != &self.target {
                return Err(CompilationError::new(
                    Stage::Target,
                    DiagnosticCode::TargetMismatch,
                    "compilation target context differs from Cargo target context",
                ));
            }
            let output = compilation.compile()?;
            fs::create_dir_all(&self.output).map_err(io_error)?;
            if fs::symlink_metadata(&destination)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                && fs::read(&destination).is_ok_and(|previous| previous == output.source.as_bytes())
            {
                return Ok(destination.clone());
            }
            let mut temporary = tempfile::Builder::new()
                .prefix(".boomerang-")
                .tempfile_in(&self.output)
                .map_err(io_error)?;
            temporary
                .write_all(output.source.as_bytes())
                .map_err(io_error)?;
            temporary.as_file().sync_all().map_err(io_error)?;
            temporary
                .persist(&destination)
                .map_err(|e| io_error(e.error))?;
            Ok(destination.clone())
        })();
        if let Err(generation_error) = &result {
            match fs::remove_file(&destination) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CompilationError::new(
                        Stage::Publish,
                        DiagnosticCode::Io,
                        format!(
                            "generation failed ({}) and stale output could not be removed: {error}",
                            generation_error
                        ),
                    )
                    .cause(error))
                }
            }
        }
        result
    }
}
fn invalid(message: impl Into<String>) -> CompilationError {
    CompilationError::new(Stage::Cargo, DiagnosticCode::InvalidInput, message)
}
fn directive_value(value: &str) -> Result<(), CompilationError> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(invalid(
            "Cargo directive values cannot contain newlines or NUL",
        ));
    }
    Ok(())
}
fn env_name(name: &str) -> Result<(), CompilationError> {
    let mut bytes = name.bytes();
    if !bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        || !bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(invalid(format!("invalid environment input name '{name}'")));
    }
    Ok(())
}
fn io_error(error: std::io::Error) -> CompilationError {
    CompilationError::new(Stage::Publish, DiagnosticCode::Io, error.to_string()).cause(error)
}
