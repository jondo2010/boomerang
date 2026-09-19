//! Build-time bounded capture settings, independent of coordination compatibility.

use serde::{Deserialize, Serialize};
use tracing_core::LevelFilter;

/// Partial bounded capture settings. Missing fields inherit deployment defaults.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct BoundedTracingSettings {
    /// Maximum enabled level; independent of `RUST_LOG`.
    #[serde(default, deserialize_with = "level_serde::deserialize_optional")]
    pub level: Option<LevelFilter>,
    /// Exact allowed targets. Replaces the inherited list; empty enables all targets.
    pub targets: Option<Vec<String>>,
    /// Retained records; zero selects loss-only capture.
    pub records: Option<u32>,
    /// Event plus inherited field occurrences per record.
    pub fields: Option<u32>,
    /// Copied event plus inherited value bytes per record.
    pub bytes: Option<u32>,
    /// Simultaneously live spans, including deferred reclamation.
    pub spans: Option<u32>,
    /// Declared fields per span.
    pub span_fields: Option<u32>,
    /// Copied value bytes per span.
    pub span_bytes: Option<u32>,
    /// Simultaneously prepared emitting threads.
    pub producers: Option<u32>,
    /// Maximum entered stack and parent ancestry depth.
    pub depth: Option<u32>,
}

/// Resolved per-process capacities and filter recorded in build/check reports.
///
/// These are element/value-byte budgets, not a total RAM estimate. Metadata,
/// scratch storage and synchronization overhead depend on the target ABI.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedTracingLimits {
    /// Maximum enabled level.
    #[serde(with = "level_serde")]
    pub level: LevelFilter,
    /// Exact allowed targets; empty enables all targets.
    pub targets: Vec<String>,
    /// Retained records.
    pub records: u32,
    /// Event plus inherited field occurrences per record.
    pub fields: u32,
    /// Copied event plus inherited value bytes per record.
    pub bytes: u32,
    /// Simultaneously live spans, including deferred reclamation.
    pub spans: u32,
    /// Declared fields per span.
    pub span_fields: u32,
    /// Copied value bytes per span.
    pub span_bytes: u32,
    /// Simultaneously prepared emitting threads.
    pub producers: u32,
    /// Maximum entered stack and parent ancestry depth.
    pub depth: u32,
}

impl Default for BoundedTracingLimits {
    fn default() -> Self {
        Self {
            level: LevelFilter::DEBUG,
            targets: vec!["boomerang::coordination".into()],
            records: 1024,
            fields: 32,
            bytes: 1024,
            spans: 64,
            span_fields: 16,
            span_bytes: 256,
            producers: 64,
            depth: 16,
        }
    }
}

/// Keep the manifest/report vocabulary stricter than native `FromStr`, which
/// also accepts numeric strings, uppercase names and an empty string.
mod level_serde {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    use tracing_core::LevelFilter;

    fn parse<E: Error>(value: String) -> Result<LevelFilter, E> {
        let level: LevelFilter = value.parse().map_err(E::custom)?;
        if level.to_string() != value {
            return Err(E::custom("expected a lowercase tracing level name"));
        }
        Ok(level)
    }

    pub(super) fn serialize<S: Serializer>(
        level: &LevelFilter,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(level)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<LevelFilter, D::Error> {
        parse(String::deserialize(deserializer)?)
    }

    pub(super) fn deserialize_optional<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<LevelFilter>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(parse)
            .transpose()
    }
}

impl BoundedTracingLimits {
    pub(crate) fn with_overrides(self, settings: Option<&BoundedTracingSettings>) -> Self {
        let Some(settings) = settings else {
            return self;
        };
        Self {
            level: settings.level.unwrap_or(self.level),
            targets: settings.targets.clone().unwrap_or(self.targets),
            records: settings.records.unwrap_or(self.records),
            fields: settings.fields.unwrap_or(self.fields),
            bytes: settings.bytes.unwrap_or(self.bytes),
            spans: settings.spans.unwrap_or(self.spans),
            span_fields: settings.span_fields.unwrap_or(self.span_fields),
            span_bytes: settings.span_bytes.unwrap_or(self.span_bytes),
            producers: settings.producers.unwrap_or(self.producers),
            depth: settings.depth.unwrap_or(self.depth),
        }
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.targets
                .iter()
                .all(|target| !target.is_empty() && target.trim() == target),
            "targets must be nonempty exact names without surrounding whitespace"
        );
        tracing_bounded::Config {
            records: self.records as usize,
            fields: self.fields as usize,
            bytes: self.bytes as usize,
            spans: self.spans as usize,
            span_fields: self.span_fields as usize,
            span_bytes: self.span_bytes as usize,
            producers: self.producers as usize,
            depth: self.depth as usize,
            ..Default::default()
        }
        .validate()?;
        Ok(())
    }
}
