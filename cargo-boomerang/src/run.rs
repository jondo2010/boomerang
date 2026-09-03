//! Host-side execution of one verified generated deployment artifact.

use std::{
    fs,
    path::Path,
    process::{Command, ExitStatus},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use crate::{
    build,
    bundle::{load_published_artifact, DeploymentDocument},
    check::analyze,
};

/// Private environment key used by generated launchers for schema-v1 summaries.
const EXECUTION_SUMMARY_ENV: &str = "BOOMERANG_EXECUTION_SUMMARY_V1";
/// Maximum accepted execution-summary file size in bytes.
const MAX_EXECUTION_SUMMARY_BYTES: u64 = 16 * 1024;

/// Aggregate work counters reported by one completed generated Federate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionStats {
    /// Number of logical tags processed by the Federate.
    processed_tags: usize,
    /// Number of reactions processed by the Federate.
    processed_reactions: usize,
    /// Number of events processed by the Federate.
    processed_events: usize,
    /// Number of ports set by the Federate.
    set_ports: usize,
    /// Number of actions scheduled by the Federate.
    scheduled_actions: usize,
}

impl ExecutionStats {
    /// Returns the number of logical tags processed by the Federate.
    pub const fn processed_tags(&self) -> usize {
        self.processed_tags
    }
    /// Returns the number of reactions processed by the Federate.
    pub const fn processed_reactions(&self) -> usize {
        self.processed_reactions
    }
    /// Returns the number of events processed by the Federate.
    pub const fn processed_events(&self) -> usize {
        self.processed_events
    }
    /// Returns the number of ports set by the Federate.
    pub const fn set_ports(&self) -> usize {
        self.set_ports
    }
    /// Returns the number of actions scheduled by the Federate.
    pub const fn scheduled_actions(&self) -> usize {
        self.scheduled_actions
    }
}

/// Final scheduling state reported by one completed generated Federate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionSummary {
    /// Aggregate work counters from the completed execution.
    stats: ExecutionStats,
    /// Last nonterminal logical tag observed by the execution.
    final_tag: boomerang_runtime::Tag,
}

impl ExecutionSummary {
    /// Returns aggregate work counters from the completed execution.
    pub const fn stats(&self) -> &ExecutionStats {
        &self.stats
    }
    /// Returns the last nonterminal logical tag observed by the execution.
    pub const fn final_tag(&self) -> boomerang_runtime::Tag {
        self.final_tag
    }
}

/// Process result and optional summary of one generated deployment execution.
#[derive(Debug)]
pub struct RunOutcome {
    /// Exit status returned by the generated executable.
    status: ExitStatus,
    /// Summary emitted only after a successful generated execution.
    summary: Option<ExecutionSummary>,
}

impl RunOutcome {
    /// Returns the generated executable's exit status.
    pub const fn status(&self) -> &ExitStatus {
        &self.status
    }
    /// Returns the execution summary when the generated executable succeeded.
    pub const fn summary(&self) -> Option<&ExecutionSummary> {
        self.summary.as_ref()
    }
}

/// Runs one host-compatible deployment through its verified published artifact.
pub fn run(workspace: impl AsRef<Path>, deployment_name: &str) -> Result<RunOutcome> {
    let analyzed = analyze(workspace.as_ref(), deployment_name)?;
    let federates = analyzed.compiled.federates();
    if federates.len() != 1 {
        bail!("generated execution currently supports exactly one local Federate");
    }
    if analyzed.resolved.deployment().coordination.is_some() {
        bail!("generated execution does not support coordination selection");
    }
    let federate = &federates[0];
    let federate_id = federate.id().as_str();
    let configuration = analyzed
        .resolved
        .deployment()
        .federates
        .get(federate_id)
        .ok_or_else(|| anyhow!("deployment has no configuration for Federate '{federate_id}'"))?;
    if federate.runtime().as_str() != "std" {
        bail!(
            "Federate '{federate_id}' selects unsupported runtime '{}'",
            federate.runtime()
        );
    }
    if configuration.target_json.is_some() {
        bail!("Federate '{federate_id}' selects unsupported custom target JSON");
    }
    if federate.target().to_string() != target_lexicon::HOST.to_string() {
        bail!(
            "Federate '{federate_id}' target '{}' is not the host target '{}'",
            federate.target(),
            target_lexicon::HOST
        );
    }

    let manifest = build(workspace, deployment_name)?;
    let published = load_published_artifact(&manifest)?;
    validate_published_host_artifact(&published.document)?;
    let directory = tempfile::tempdir().context("failed to create execution-summary directory")?;
    let summary_path = directory.path().join("summary.json");
    let status = Command::new(&published.executable)
        .env(EXECUTION_SUMMARY_ENV, &summary_path)
        .status()
        .with_context(|| format!("failed to launch {}", published.executable.display()))?;
    if status.success() {
        Ok(RunOutcome {
            status,
            summary: Some(read_execution_summary(&summary_path)?),
        })
    } else {
        Ok(RunOutcome {
            status,
            summary: None,
        })
    }
}

/// Confirms that a validated bundle remains runnable by the local host process.
fn validate_published_host_artifact(document: &DeploymentDocument) -> Result<()> {
    if document.federates.len() != 1 {
        bail!("published artifact requires exactly one local Federate");
    }
    if document.coordination.backend != "local" || document.coordination.protocol.is_some() {
        bail!("published artifact selects unsupported coordination");
    }
    let federate = &document.federates[0];
    if federate.runtime != "std" {
        bail!(
            "published artifact Federate '{}' selects unsupported runtime",
            federate.id
        );
    }
    if federate.target_json_hash.is_some() {
        bail!(
            "published artifact Federate '{}' selects custom target JSON",
            federate.id
        );
    }
    if federate.target != target_lexicon::HOST.to_string() {
        bail!(
            "published artifact Federate '{}' is not built for the host target",
            federate.id
        );
    }
    Ok(())
}

/// Schema-v1 summary document emitted only through the private out-of-band file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionSummaryDocumentV1 {
    /// Protocol schema version.
    schema: u32,
    /// Aggregate scheduling counters encoded as decimal strings.
    stats: ExecutionStatsDocumentV1,
    /// Final logical tag encoded as decimal strings.
    final_tag: FinalTagDocumentV1,
}

/// Schema-v1 aggregate scheduling counters encoded as decimal strings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionStatsDocumentV1 {
    /// Processed logical tag count.
    processed_tags: String,
    /// Processed reaction count.
    processed_reactions: String,
    /// Processed event count.
    processed_events: String,
    /// Set port count.
    set_ports: String,
    /// Scheduled action count.
    scheduled_actions: String,
}

/// Schema-v1 final logical tag encoded as decimal strings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinalTagDocumentV1 {
    /// Signed logical offset in nanoseconds.
    offset_nanos: String,
    /// Superdense microstep count.
    microstep: String,
}

/// Reads, validates, and decodes one schema-v1 execution-summary file.
fn read_execution_summary(path: &Path) -> Result<ExecutionSummary> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("{} is not a regular execution summary file", path.display());
    }
    if metadata.len() > MAX_EXECUTION_SUMMARY_BYTES {
        bail!("execution summary exceeds {MAX_EXECUTION_SUMMARY_BYTES} bytes");
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let document: ExecutionSummaryDocumentV1 = serde_json::from_slice(&bytes)
        .map_err(|error| anyhow!("failed to decode {}: {error}", path.display()))?;
    if document.schema != 1 {
        bail!("unsupported execution summary schema {}", document.schema);
    }
    let stats = document.stats;
    let final_tag = document.final_tag;
    let offset_nanos = parse_i128("offset_nanos", &final_tag.offset_nanos)?;
    Ok(ExecutionSummary {
        stats: ExecutionStats {
            processed_tags: parse_usize("processed_tags", &stats.processed_tags)?,
            processed_reactions: parse_usize("processed_reactions", &stats.processed_reactions)?,
            processed_events: parse_usize("processed_events", &stats.processed_events)?,
            set_ports: parse_usize("set_ports", &stats.set_ports)?,
            scheduled_actions: parse_usize("scheduled_actions", &stats.scheduled_actions)?,
        },
        final_tag: boomerang_runtime::Tag::new(
            boomerang_runtime::Duration::nanoseconds_i128(offset_nanos),
            parse_usize("microstep", &final_tag.microstep)?,
        ),
    })
}

/// Parses one unsigned decimal protocol value within the host `usize` range.
fn parse_usize(name: &str, value: &str) -> Result<usize> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("invalid decimal {name}");
    }
    value
        .parse()
        .map_err(|_| anyhow!("{name} is outside host usize"))
}

/// Parses one signed decimal protocol value within the `i128` range.
fn parse_i128(name: &str, value: &str) -> Result<i128> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("invalid decimal {name}");
    }
    value.parse().map_err(|_| anyhow!("{name} is outside i128"))
}

#[cfg(test)]
mod tests {
    use super::read_execution_summary;
    use std::{fs, path::Path};
    const VALID_SUMMARY: &str = r#"{"schema":1,"stats":{"processed_tags":"1","processed_reactions":"2","processed_events":"3","set_ports":"4","scheduled_actions":"5"},"final_tag":{"offset_nanos":"6","microstep":"7"}}"#;
    fn write_summary(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
    }
    #[test]
    fn execution_summary_decoder_rejects_invalid_protocol_files() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.json");
        let error = read_execution_summary(&missing).unwrap_err();
        assert!(error.to_string().contains("failed to inspect"), "{error:#}");
        let cases = [
            (
                "unsupported-schema",
                r#"{"schema":2,"stats":{"processed_tags":"1","processed_reactions":"2","processed_events":"3","set_ports":"4","scheduled_actions":"5"},"final_tag":{"offset_nanos":"6","microstep":"7"}}"#,
                "unsupported execution summary schema 2",
            ),
            (
                "unknown-field",
                r#"{"schema":1,"stats":{"processed_tags":"1","processed_reactions":"2","processed_events":"3","set_ports":"4","scheduled_actions":"5","unexpected":"6"},"final_tag":{"offset_nanos":"6","microstep":"7"}}"#,
                "unknown field",
            ),
            (
                "non-decimal",
                r#"{"schema":1,"stats":{"processed_tags":"one","processed_reactions":"2","processed_events":"3","set_ports":"4","scheduled_actions":"5"},"final_tag":{"offset_nanos":"6","microstep":"7"}}"#,
                "invalid decimal processed_tags",
            ),
            (
                "counter-overflow",
                r#"{"schema":1,"stats":{"processed_tags":"340282366920938463463374607431768211456","processed_reactions":"2","processed_events":"3","set_ports":"4","scheduled_actions":"5"},"final_tag":{"offset_nanos":"6","microstep":"7"}}"#,
                "processed_tags is outside host usize",
            ),
            (
                "offset-overflow",
                r#"{"schema":1,"stats":{"processed_tags":"1","processed_reactions":"2","processed_events":"3","set_ports":"4","scheduled_actions":"5"},"final_tag":{"offset_nanos":"170141183460469231731687303715884105728","microstep":"7"}}"#,
                "offset_nanos is outside i128",
            ),
        ];
        for (name, contents, expected) in cases {
            let path = directory.path().join(format!("{name}.json"));
            write_summary(&path, contents);
            let error = read_execution_summary(&path).unwrap_err();
            assert!(error.to_string().contains(expected), "{name}: {error:#}");
        }
        let oversized = directory.path().join("oversized.json");
        write_summary(&oversized, &"x".repeat(16 * 1024 + 1));
        let error = read_execution_summary(&oversized).unwrap_err();
        assert!(
            error.to_string().contains("exceeds 16384 bytes"),
            "{error:#}"
        );
    }
    #[cfg(unix)]
    #[test]
    fn execution_summary_decoder_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.json");
        write_summary(&target, VALID_SUMMARY);
        let link = directory.path().join("summary.json");
        symlink(&target, &link).unwrap();
        let error = read_execution_summary(&link).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not a regular execution summary file"),
            "{error:#}"
        );
    }
}
