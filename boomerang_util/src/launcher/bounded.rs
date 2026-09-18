//! Hosted shutdown export; no serialization or I/O runs in producer callbacks.
use serde_json::{json, Value as Json};
use std::io::{self, Write};
use tracing_bounded::{CaptureHandle, LossCount, OwnedField, ProducerGuard, Value};

pub(super) struct Guard {
    capture: CaptureHandle,
    _producer: ProducerGuard,
}

pub(super) fn init(mut config: tracing_bounded::Config) -> io::Result<Guard> {
    config.level = tracing::level_filters::LevelFilter::DEBUG;
    config.targets = &["boomerang::coordination"];
    config.references = std::num::NonZeroU16::new(1024).unwrap();
    config.loss_ceiling = std::num::NonZeroU16::new(1024).unwrap();
    let (subscriber, capture) =
        tracing_bounded::BoundedSubscriber::new(config).map_err(io::Error::other)?;
    let producer = capture.prepare_current_thread().map_err(io::Error::other)?;
    tracing::subscriber::set_global_default(subscriber).map_err(io::Error::other)?;
    Ok(Guard {
        capture,
        _producer: producer,
    })
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Err(error) = write_capture(&mut io::stderr().lock(), &self.capture) {
            let _ = writeln!(io::stderr().lock(), "bounded trace export failed: {error}");
        }
    }
}

fn fields(fields: &[OwnedField]) -> Json {
    fields
        .iter()
        .map(|field| {
            let value = match &field.value {
                Value::Bool(v) => json!(v),
                Value::I64(v) => json!(v),
                Value::U64(v) => json!(v),
                Value::I128(v) => json!(v.to_string()),
                Value::U128(v) => json!(v.to_string()),
                Value::F64(v) if v.is_finite() => json!(v),
                Value::F64(v) => json!({ "f64_bits": format!("{:016x}", v.to_bits()) }),
                Value::Str(v) => json!(v),
                Value::Bytes(v) => json!(v),
            };
            (field.name.to_owned(), value)
        })
        .collect()
}

fn count(count: LossCount) -> Json {
    json!({ "value": count.value, "saturated": count.saturated })
}

fn line(writer: &mut impl Write, value: Json) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)
}

pub(super) fn write_capture(writer: &mut impl Write, capture: &CaptureHandle) -> io::Result<()> {
    for record in capture.snapshot() {
        let scopes: Vec<_> = record
            .scopes
            .iter()
            .map(|scope| {
                json!({
                    "name": scope.metadata.name(), "fields": fields(&scope.fields)
                })
            })
            .collect();
        line(
            writer,
            json!({
                "trace_schema": 1, "kind": "record", "pid": std::process::id(),
                "target": record.metadata.target(), "level": record.metadata.level().as_str(),
                "fields": fields(&record.fields), "scopes": scopes
            }),
        )?;
    }
    let loss = capture.loss();
    let lifecycle = capture.lifecycle_loss();
    line(
        writer,
        json!({
            "trace_schema": 1, "kind": "loss", "pid": std::process::id(),
            "loss": {
                "no_record_capacity": count(loss.no_record_capacity),
                "contention": count(loss.contention),
                "invalid_context": count(loss.invalid_context),
                "field_limit": count(loss.field_limit), "byte_limit": count(loss.byte_limit),
                "unsupported_value": count(loss.unsupported_value),
                "overwritten_records": count(loss.overwritten_records)
            },
            "lifecycle_loss": {
                "span_admission": count(lifecycle.span_admission),
                "span_update": count(lifecycle.span_update),
                "producer_admission": count(lifecycle.producer_admission),
                "invalid_context_transitions": count(lifecycle.invalid_context_transitions),
                "unsupported_context_operation": count(lifecycle.unsupported_context_operation)
            }
        }),
    )
}
