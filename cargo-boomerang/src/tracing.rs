//! Build-time bounded capture limits, independent of coordination compatibility.

use serde::{Deserialize, Serialize};

/// Partial bounded capture settings. Missing fields inherit deployment defaults.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct BoundedTracingSettings {
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

/// Resolved per-process capacities recorded in build/check reports.
///
/// These are element/value-byte budgets, not a total RAM estimate. Metadata,
/// scratch storage and synchronization overhead depend on the target ABI.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedTracingLimits {
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

impl BoundedTracingLimits {
    pub(crate) fn with_overrides(self, settings: Option<&BoundedTracingSettings>) -> Self {
        let Some(settings) = settings else {
            return self;
        };
        Self {
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

    pub(crate) fn validate(self) -> Result<(), tracing_bounded::BuildError> {
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
        .validate()
    }
}
