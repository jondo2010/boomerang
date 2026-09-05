//! Cargo-compatible command progress and nested-process configuration.

use std::{
    env, fmt,
    io::{self, Write},
    process::{Command, Output},
};

#[cfg(not(windows))]
use std::io::IsTerminal;

use anyhow::{Context, Result};

/// Color policy shared by cargo-boomerang status lines and nested Cargo commands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorChoice {
    /// Use color only when stderr is a terminal.
    #[default]
    Auto,
    /// Always emit ANSI color escapes.
    Always,
    /// Never emit ANSI color escapes.
    Never,
}

impl ColorChoice {
    /// Reads Cargo's `CARGO_TERM_COLOR` convention, falling back to automatic detection.
    pub fn from_cargo_env() -> Self {
        match env::var("CARGO_TERM_COLOR").as_deref() {
            Ok("always") => Self::Always,
            Ok("never") => Self::Never,
            _ => Self::Auto,
        }
    }

    fn as_cargo_value(self) -> &'static str {
        match self {
            Self::Auto if cargo_auto_color_enabled() => "always",
            Self::Auto => "never",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    fn stream_choice(self) -> anstream::ColorChoice {
        match self {
            Self::Auto => anstream::ColorChoice::Auto,
            Self::Always => anstream::ColorChoice::Always,
            Self::Never => anstream::ColorChoice::Never,
        }
    }
}

/// Output policy for one cargo-boomerang command invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    quiet: bool,
    verbosity: u8,
    color: ColorChoice,
    configure_cargo: bool,
}

impl CommandOutput {
    /// Creates a CLI output policy using Cargo-compatible quiet, verbose, and color choices.
    pub const fn new(quiet: bool, verbosity: u8, color: ColorChoice) -> Self {
        Self {
            quiet,
            verbosity,
            color,
            configure_cargo: true,
        }
    }

    /// Creates a silent policy for library entry points without changing nested Cargo behavior.
    pub(crate) const fn silent() -> Self {
        Self {
            quiet: true,
            verbosity: 0,
            color: ColorChoice::Auto,
            configure_cargo: false,
        }
    }

    /// Writes and flushes one deterministic Cargo-style status line to stderr.
    pub(crate) fn status(self, phase: Phase, message: impl fmt::Display) -> Result<()> {
        if self.quiet {
            return Ok(());
        }
        let mut stderr = anstream::AutoStream::new(io::stderr(), self.color.stream_choice());
        writeln!(stderr, "\x1b[1;32m{phase:>12}\x1b[0m {message}")?;
        stderr.flush().context("failed to flush command progress")
    }

    /// Applies the selected Cargo verbosity and color conventions to a nested command.
    pub(crate) fn configure(self, command: &mut Command) {
        if !self.configure_cargo {
            return;
        }
        command.env("CARGO_TERM_COLOR", self.color.as_cargo_value());
        if self.verbosity == 0 {
            command.arg("--quiet");
        } else {
            command.arg(format!("-{}", "v".repeat(usize::from(self.verbosity))));
        }
    }

    /// Adds this policy's common options to a Cargo subcommand argument vector.
    pub(crate) fn extend_cargo_options(self, arguments: &mut Vec<String>) {
        if !self.configure_cargo {
            return;
        }
        arguments.extend([String::from("--color"), self.color.as_cargo_value().into()]);
        if self.verbosity == 0 {
            arguments.push(String::from("--quiet"));
        } else {
            arguments.push(format!("-{}", "v".repeat(usize::from(self.verbosity))));
        }
    }

    /// Returns whether successful nested Cargo stderr is shown directly to the CLI user.
    pub(crate) const fn shows_successful_cargo_stderr(self) -> bool {
        self.configure_cargo && !self.quiet
    }

    /// Returns whether successful nested Cargo stderr is retained for a library caller.
    pub(crate) const fn retains_successful_cargo_stderr(self) -> bool {
        !self.configure_cargo
    }

    /// Forwards successful nested Cargo output after its normal-verbosity noise is suppressed.
    pub(crate) fn forward_cargo_stderr(self, output: &Output) -> Result<()> {
        if !self.shows_successful_cargo_stderr()
            || !output.status.success()
            || output.stderr.is_empty()
        {
            return Ok(());
        }
        let mut stderr = anstream::AutoStream::new(io::stderr(), self.color.stream_choice());
        stderr
            .write_all(&output.stderr)
            .context("failed to forward nested Cargo output")?;
        stderr
            .flush()
            .context("failed to flush nested Cargo output")
    }
}

/// Resolves Cargo's automatic child color against cargo-boomerang's final stderr.
fn cargo_auto_color_enabled() -> bool {
    #[cfg(windows)]
    {
        false
    }
    #[cfg(not(windows))]
    {
        io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none()
    }
}

/// Stable action labels used in human-readable progress output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    /// Resolve and inspect a deployment.
    Analyzing,
    /// Generate a descriptor driver or launcher.
    Generating,
    /// Execute a nested Cargo build.
    Building,
    /// Validate compiler or published-artifact state.
    Validating,
    /// Assemble an immutable deployment bundle.
    Bundling,
    /// Atomically publish a report or bundle.
    Publishing,
    /// Execute the generated deployment.
    Running,
}

impl fmt::Display for Phase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Analyzing => "Analyzing",
            Self::Generating => "Generating",
            Self::Building => "Building",
            Self::Validating => "Validating",
            Self::Bundling => "Bundling",
            Self::Publishing => "Publishing",
            Self::Running => "Running",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorChoice, CommandOutput};

    #[test]
    fn successful_nested_cargo_stderr_policy_distinguishes_cli_and_library_calls() {
        let default = CommandOutput::new(false, 0, ColorChoice::Never);
        assert!(default.shows_successful_cargo_stderr());
        assert!(!default.retains_successful_cargo_stderr());

        let quiet = CommandOutput::new(true, 0, ColorChoice::Never);
        assert!(!quiet.shows_successful_cargo_stderr());
        assert!(!quiet.retains_successful_cargo_stderr());

        let verbose = CommandOutput::new(false, 1, ColorChoice::Never);
        assert!(verbose.shows_successful_cargo_stderr());
        assert!(!verbose.retains_successful_cargo_stderr());

        let library = CommandOutput::silent();
        assert!(!library.shows_successful_cargo_stderr());
        assert!(library.retains_successful_cargo_stderr());
    }
}
