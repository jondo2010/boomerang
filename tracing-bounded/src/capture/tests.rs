use std::{fmt, num::NonZeroU16, sync::mpsc, thread, time::Duration};

use tracing_core::{
    span, subscriber::Interest, Callsite, Event, Level, LevelFilter, Metadata, Subscriber,
};

use super::{
    record::{OwnedField, Value},
    BuildError, Capture, Inspector, Limits, LossSnapshot, Record,
};

pub(super) struct CaptureSubscriber {
    pub(super) capture: Capture,
}

impl Subscriber for CaptureSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "capture-test" && *metadata.level() <= Level::INFO
    }

    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::INFO)
    }

    fn event(&self, event: &Event<'_>) {
        self.capture.event(event);
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        panic!("spans are not supported by the root-only test subscriber")
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {
        panic!("spans are not supported by the root-only test subscriber")
    }

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {
        panic!("spans are not supported by the root-only test subscriber")
    }

    fn enter(&self, _: &span::Id) {
        panic!("spans are not supported by the root-only test subscriber")
    }

    fn exit(&self, _: &span::Id) {
        panic!("spans are not supported by the root-only test subscriber")
    }
}

fn setup(
    records: usize,
    fields: usize,
    bytes: usize,
    ceiling: u16,
) -> (tracing::Dispatch, Inspector) {
    let limits = Limits {
        records,
        fields,
        bytes,
        loss_ceiling: NonZeroU16::new(ceiling).expect("test ceiling must be nonzero"),
    };
    let (capture, inspect) = Capture::new(limits).expect("test limits must be valid");
    let dispatch = tracing::Dispatch::new(CaptureSubscriber { capture });
    (dispatch, inspect)
}

fn assert_build_error(limits: Limits, expected: BuildError) {
    match Capture::new(limits) {
        Err(actual) => assert_eq!(actual, expected),
        Ok(_) => panic!("invalid limits unexpectedly constructed a capture"),
    }
}

struct Bomb;

impl fmt::Debug for Bomb {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("debug formatting must not run")
    }
}

impl fmt::Display for Bomb {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("display formatting must not run")
    }
}

impl std::error::Error for Bomb {}

fn field(name: &'static str, value: Value) -> OwnedField {
    OwnedField { name, value }
}

fn only_record(inspect: &Inspector) -> Record {
    let mut records = inspect.snapshot();
    assert_eq!(records.len(), 1, "expected exactly one retained record");
    records.pop().unwrap()
}

fn numbers(records: &[Record]) -> Vec<u64> {
    records
        .iter()
        .map(|record| match record.fields.as_slice() {
            [field] if field.name == "n" => match field.value {
                Value::U64(value) => value,
                ref other => panic!("field n had unexpected value {other:?}"),
            },
            fields => panic!("record had unexpected fields {fields:?}"),
        })
        .collect()
}

#[test]
fn rejected_event_preserves_old_output_and_success_overwrites_oldest() {
    let (dispatch, inspect) = setup(2, 1, 2, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, n = 1u64);
        tracing::info!(target: "capture-test", parent: None, n = 2u64);
        tracing::info!(target: "capture-test", parent: None, n = ?Bomb);
    });
    assert_eq!(numbers(&inspect.snapshot()), vec![1, 2]);
    assert_eq!(inspect.loss().unsupported_value.value, 1);
    assert_eq!(inspect.loss().overwritten_records.value, 0);

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, n = 3u64);
    });
    assert_eq!(numbers(&inspect.snapshot()), vec![2, 3]);
    assert_eq!(inspect.loss().overwritten_records.value, 1);
}

#[test]
fn manual_positional_values_retain_declared_field_identities() {
    let (dispatch, inspect) = setup(1, 2, 1, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        let callsite = tracing::callsite! {
            name: "manual positional",
            kind: tracing_core::metadata::Kind::EVENT,
            target: "capture-test",
            level: Level::INFO,
            fields: count, ready
        };
        let metadata = callsite.metadata();
        let values: [Option<&dyn tracing_core::field::Value>; 2] = [Some(&7u64), Some(&true)];
        Event::child_of(None, metadata, &metadata.fields().value_set_all(&values));
    });

    let record = only_record(&inspect);
    assert_eq!(record.metadata.name(), "manual positional");
    assert_eq!(
        record.fields,
        vec![
            field("count", Value::U64(7)),
            field("ready", Value::Bool(true))
        ]
    );
    assert_eq!(inspect.loss(), LossSnapshot::default());
}

#[test]
fn manual_sparse_reordered_values_retain_supplied_field_identities() {
    let (dispatch, inspect) = setup(1, 4, 1, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        let callsite = tracing::callsite! {
            name: "manual sparse",
            kind: tracing_core::metadata::Kind::EVENT,
            target: "capture-test",
            level: Level::INFO,
            fields: count, absent, ready, omitted
        };
        let metadata = callsite.metadata();
        let fields = metadata.fields();
        let count = fields.field("count").unwrap();
        let absent = fields.field("absent").unwrap();
        let ready = fields.field("ready").unwrap();
        let values = [
            (&ready, Some(&true as &dyn tracing_core::field::Value)),
            (&absent, None),
            (&count, Some(&7u64 as &dyn tracing_core::field::Value)),
        ];
        Event::child_of(None, metadata, &fields.value_set(&values));
    });

    let record = only_record(&inspect);
    assert_eq!(record.metadata.name(), "manual sparse");
    assert_eq!(
        record.fields,
        vec![
            field("ready", Value::Bool(true)),
            field("count", Value::U64(7))
        ]
    );
    assert_eq!(inspect.loss(), LossSnapshot::default());
}

#[test]
fn native_booleans_and_integers_are_not_narrowed() {
    let (dispatch, inspect) = setup(1, 16, 1, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(
            target: "capture-test",
            parent: None,
            truth = true,
            falsity = false,
            i8_min = i8::MIN,
            i16_max = i16::MAX,
            i32_min = i32::MIN,
            i64_min = i64::MIN,
            i64_max = i64::MAX,
            u8_max = u8::MAX,
            u16_max = u16::MAX,
            u32_max = u32::MAX,
            u64_min = u64::MIN,
            u64_max = u64::MAX,
            i128_min = i128::MIN,
            i128_max = i128::MAX,
            u128_min = u128::MIN,
            u128_max = u128::MAX,
        );
    });

    assert_eq!(
        only_record(&inspect).fields,
        vec![
            field("truth", Value::Bool(true)),
            field("falsity", Value::Bool(false)),
            field("i8_min", Value::I64(i64::from(i8::MIN))),
            field("i16_max", Value::I64(i64::from(i16::MAX))),
            field("i32_min", Value::I64(i64::from(i32::MIN))),
            field("i64_min", Value::I64(i64::MIN)),
            field("i64_max", Value::I64(i64::MAX)),
            field("u8_max", Value::U64(u64::from(u8::MAX))),
            field("u16_max", Value::U64(u64::from(u16::MAX))),
            field("u32_max", Value::U64(u64::from(u32::MAX))),
            field("u64_min", Value::U64(u64::MIN)),
            field("u64_max", Value::U64(u64::MAX)),
            field("i128_min", Value::I128(i128::MIN)),
            field("i128_max", Value::I128(i128::MAX)),
            field("u128_min", Value::U128(u128::MIN)),
            field("u128_max", Value::U128(u128::MAX)),
        ]
    );
}

#[test]
fn non_finite_and_signed_zero_floats_preserve_bits() {
    let nan = f64::from_bits(0x7ff8_0000_0000_0042);
    let values = [nan, f64::INFINITY, f64::NEG_INFINITY, -0.0];
    let (dispatch, inspect) = setup(1, 4, 1, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(
            target: "capture-test",
            parent: None,
            nan = values[0],
            positive_infinity = values[1],
            negative_infinity = values[2],
            negative_zero = values[3],
        );
    });

    let actual: Vec<_> = only_record(&inspect)
        .fields
        .into_iter()
        .map(|field| match field.value {
            Value::F64(value) => value.to_bits(),
            value => panic!("expected float for {}, got {value:?}", field.name),
        })
        .collect();
    assert_eq!(actual, values.map(f64::to_bits));
}

#[test]
fn borrowed_utf8_and_bytes_are_copied_before_return() {
    let mut text = String::from("Grüße");
    let mut bytes = vec![0u8, 1, 0xfe, 0xff];
    let (dispatch, inspect) = setup(1, 2, text.len() + bytes.len(), 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(
            target: "capture-test",
            parent: None,
            text = text.as_str(),
            bytes = bytes.as_slice(),
        );
    });
    text.replace_range(.., "xxxxxxx");
    bytes.fill(7);

    assert_eq!(
        only_record(&inspect).fields,
        vec![
            field("text", Value::Str("Grüße".to_owned())),
            field("bytes", Value::Bytes(vec![0, 1, 0xfe, 0xff])),
        ]
    );
}

#[test]
fn field_and_byte_fenceposts_accept_exact_and_empty_values() {
    let (dispatch, inspect) = setup(2, 2, 5, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, a = "ab", b = &[1u8, 2, 3][..]);
        tracing::info!(target: "capture-test", parent: None, a = "", b = &[] as &[u8]);
    });
    let records = inspect.snapshot();
    assert_eq!(records[0].fields[0].value, Value::Str("ab".to_owned()));
    assert_eq!(records[0].fields[1].value, Value::Bytes(vec![1, 2, 3]));
    assert_eq!(records[1].fields[0].value, Value::Str(String::new()));
    assert_eq!(records[1].fields[1].value, Value::Bytes(Vec::new()));
    assert_eq!(inspect.loss(), LossSnapshot::default());
}

#[test]
fn byte_budget_is_shared_and_one_over_rejects_whole_event() {
    let (dispatch, inspect) = setup(1, 2, 4, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, a = "ab", b = &[1u8, 2, 3][..]);
    });
    assert!(inspect.snapshot().is_empty());
    assert_eq!(inspect.loss().byte_limit.value, 1);
    assert_eq!(inspect.loss().unsupported_value.value, 0);
}

#[test]
fn declared_field_excess_rejects_before_visiting_values() {
    let (dispatch, inspect) = setup(1, 1, 8, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(
            target: "capture-test",
            parent: None,
            empty = tracing::field::Empty,
            bomb = ?Bomb,
        );
    });
    assert!(inspect.snapshot().is_empty());
    assert_eq!(inspect.loss().field_limit.value, 1);
    assert_eq!(inspect.loss().unsupported_value.value, 0);
}

#[test]
fn first_native_visitation_error_is_the_only_rejection() {
    let (byte_dispatch, byte_inspect) = setup(1, 2, 1, 20);
    tracing::dispatcher::with_default(&byte_dispatch, || {
        tracing::info!(target: "capture-test", parent: None, first = "xx", second = ?Bomb);
    });
    assert_eq!(byte_inspect.loss().byte_limit.value, 1);
    assert_eq!(byte_inspect.loss().unsupported_value.value, 0);

    let (unsupported_dispatch, unsupported_inspect) = setup(1, 2, 1, 20);
    tracing::dispatcher::with_default(&unsupported_dispatch, || {
        tracing::info!(target: "capture-test", parent: None, first = ?Bomb, second = "xx");
    });
    assert_eq!(unsupported_inspect.loss().unsupported_value.value, 1);
    assert_eq!(unsupported_inspect.loss().byte_limit.value, 0);
}

#[test]
fn debug_display_error_and_message_reject_without_formatting() {
    let (dispatch, inspect) = setup(4, 1, 8, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, value = ?Bomb);
        tracing::info!(target: "capture-test", parent: None, value = %Bomb);
        tracing::info!(target: "capture-test", parent: None, error = &Bomb as &dyn std::error::Error);
        tracing::info!(target: "capture-test", parent: None, "{}", Bomb);
    });
    assert!(inspect.snapshot().is_empty());
    assert_eq!(inspect.loss().unsupported_value.value, 4);
}

#[test]
fn every_rejection_kind_preserves_a_full_ring() {
    let (dispatch, inspect) = setup(1, 1, 1, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, n = 7u64);
        tracing::info!(target: "capture-test", parent: None, a = 1u64, b = 2u64);
        tracing::info!(target: "capture-test", parent: None, text = "xx");
        tracing::info!(target: "capture-test", parent: None, value = ?Bomb);
        tracing::info!(target: "capture-test", n = 8u64);
    });
    assert_eq!(numbers(&inspect.snapshot()), vec![7]);
    let loss = inspect.loss();
    assert_eq!(loss.field_limit.value, 1);
    assert_eq!(loss.byte_limit.value, 1);
    assert_eq!(loss.unsupported_value.value, 1);
    assert_eq!(loss.invalid_context.value, 1);
    assert_eq!(loss.overwritten_records.value, 0);
}

#[test]
fn ring_wrap_retains_last_records_without_changing_prior_snapshot() {
    let (dispatch, inspect) = setup(3, 1, 1, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, n = 1u64);
        tracing::info!(target: "capture-test", parent: None, n = 2u64);
        tracing::info!(target: "capture-test", parent: None, n = 3u64);
    });
    let before_wrap = inspect.snapshot();
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, n = 4u64);
        tracing::info!(target: "capture-test", parent: None, n = 5u64);
        tracing::info!(target: "capture-test", parent: None, n = 6u64);
        tracing::info!(target: "capture-test", parent: None, n = 7u64);
    });
    assert_eq!(numbers(&before_wrap), vec![1, 2, 3]);
    assert_eq!(numbers(&inspect.snapshot()), vec![5, 6, 7]);
    assert_eq!(inspect.loss().overwritten_records.value, 4);
}

#[test]
fn native_metadata_and_declared_field_names_are_retained() {
    let (dispatch, inspect) = setup(1, 2, 1, 20);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, alpha = 1u64, beta = false);
    });
    let record = only_record(&inspect);
    assert_eq!(record.metadata.target(), "capture-test");
    assert_eq!(*record.metadata.level(), Level::INFO);
    assert_eq!(
        record
            .fields
            .iter()
            .map(|field| field.name)
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
}

fn emit_while_storage_is_held(
    dispatch: &tracing::Dispatch,
    inspect: &Inspector,
    emit: impl FnOnce() + Send + 'static,
    assert_loss: impl FnOnce(LossSnapshot),
) {
    let storage = inspect
        .shared
        .storage
        .lock()
        .expect("test storage mutex must not be poisoned");
    let worker_dispatch = dispatch.clone();
    let (completed_tx, completed_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        tracing::dispatcher::with_default(&worker_dispatch, emit);
        completed_tx
            .send(())
            .expect("test completion receiver must remain alive");
    });

    let completion = completed_rx.recv_timeout(Duration::from_secs(1));
    let loss_while_held = inspect.loss();
    drop(storage);
    let joined = worker.join();

    assert!(joined.is_ok(), "emission worker panicked: {joined:?}");
    assert!(
        completion.is_ok(),
        "emission did not complete while storage was held: {completion:?}"
    );
    assert_loss(loss_while_held);
}

#[test]
fn zero_records_reports_loss_without_formatting() {
    let (dispatch, inspect) = setup(0, 4, 8, 3);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, x = ?Bomb);
    });
    assert!(inspect.snapshot().is_empty());
    assert_eq!(inspect.loss().no_record_capacity.value, 1);
    assert!(!inspect.loss().no_record_capacity.saturated);
    assert_eq!(inspect.loss().unsupported_value.value, 0);
}

#[test]
fn saturation_is_visible_and_never_wraps() {
    let (dispatch, inspect) = setup(0, 4, 8, 3);
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..10 {
            tracing::info!(target: "capture-test", parent: None, x = 7u64);
        }
    });
    let loss = inspect.loss();
    assert_eq!(loss.no_record_capacity.value, 3);
    assert!(loss.no_record_capacity.saturated);
}

#[test]
fn a_different_target_is_filtered_without_loss_or_formatting() {
    let (dispatch, inspect) = setup(1, 4, 8, 3);
    emit_while_storage_is_held(
        &dispatch,
        &inspect,
        || tracing::info!(target: "not-capture-test", parent: None, x = ?Bomb),
        |loss| assert_eq!(loss, LossSnapshot::default()),
    );
}

#[test]
fn a_more_verbose_level_is_filtered_without_loss_or_formatting() {
    let (dispatch, inspect) = setup(1, 4, 8, 3);
    emit_while_storage_is_held(
        &dispatch,
        &inspect,
        || tracing::debug!(target: "capture-test", parent: None, x = ?Bomb),
        |loss| assert_eq!(loss, LossSnapshot::default()),
    );
}

#[test]
fn zero_capacity_precedes_invalid_context() {
    let (dispatch, inspect) = setup(0, 4, 8, 3);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", x = 7u64);
    });
    let loss = inspect.loss();
    assert_eq!(loss.no_record_capacity.value, 1);
    assert_eq!(loss.invalid_context.value, 0);
}

#[test]
fn zero_capacity_precedes_contention() {
    let (dispatch, inspect) = setup(0, 4, 8, 3);
    emit_while_storage_is_held(
        &dispatch,
        &inspect,
        || tracing::info!(target: "capture-test", parent: None, x = 7u64),
        |loss| {
            assert_eq!(loss.no_record_capacity.value, 1);
            assert_eq!(loss.contention.value, 0);
        },
    );
}

#[test]
fn explicit_root_is_admitted_without_rejection() {
    let (dispatch, inspect) = setup(1, 4, 8, 3);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: None, x = 7u64);
    });
    assert_eq!(inspect.loss(), LossSnapshot::default());
}

#[test]
fn contextual_event_is_rejected_as_invalid_context() {
    let (dispatch, inspect) = setup(1, 4, 8, 3);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", x = 7u64);
    });
    let loss = inspect.loss();
    assert_eq!(loss.invalid_context.value, 1);
    assert_eq!(loss.contention.value, 0);
}

#[test]
fn explicit_parent_is_rejected_as_invalid_context() {
    let (dispatch, inspect) = setup(1, 4, 8, 3);
    let parent = span::Id::from_u64(17);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(target: "capture-test", parent: &parent, x = 7u64);
    });
    let loss = inspect.loss();
    assert_eq!(loss.invalid_context.value, 1);
    assert_eq!(loss.contention.value, 0);
}

#[test]
fn inaccessible_storage_rejects_without_waiting() {
    let (dispatch, inspect) = setup(1, 4, 8, 3);
    emit_while_storage_is_held(
        &dispatch,
        &inspect,
        || tracing::info!(target: "capture-test", parent: None, x = 7u64),
        |loss| {
            assert_eq!(loss.contention.value, 1);
            assert_eq!(loss.invalid_context.value, 0);
        },
    );
}

#[test]
fn contention_precedes_invalid_context() {
    let (dispatch, inspect) = setup(1, 4, 8, 3);
    emit_while_storage_is_held(
        &dispatch,
        &inspect,
        || tracing::info!(target: "capture-test", x = 7u64),
        |loss| {
            assert_eq!(loss.contention.value, 1);
            assert_eq!(loss.invalid_context.value, 0);
        },
    );
}

#[test]
fn concurrent_zero_capacity_emissions_are_exact_below_ceiling() {
    let (dispatch, inspect) = setup(0, 4, 8, 17);
    let dispatches: Vec<_> = (0..3).map(|_| dispatch.clone()).collect();
    let workers: Vec<_> = dispatches
        .into_iter()
        .map(|worker_dispatch| {
            thread::spawn(move || {
                tracing::dispatcher::with_default(&worker_dispatch, || {
                    for _ in 0..4 {
                        tracing::info!(target: "capture-test", parent: None, x = 7u64);
                    }
                });
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("emission worker must not panic");
    }

    let loss = inspect.loss();
    assert_eq!(loss.no_record_capacity.value, 12);
    assert!(!loss.no_record_capacity.saturated);
}

#[test]
fn concurrent_zero_capacity_emissions_saturate_above_ceiling() {
    let (dispatch, inspect) = setup(0, 4, 8, 7);
    let dispatches: Vec<_> = (0..4).map(|_| dispatch.clone()).collect();
    let workers: Vec<_> = dispatches
        .into_iter()
        .map(|worker_dispatch| {
            thread::spawn(move || {
                tracing::dispatcher::with_default(&worker_dispatch, || {
                    for _ in 0..5 {
                        tracing::info!(target: "capture-test", parent: None, x = 7u64);
                    }
                });
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("emission worker must not panic");
    }

    let loss = inspect.loss();
    assert_eq!(loss.no_record_capacity.value, 7);
    assert!(loss.no_record_capacity.saturated);
}

#[test]
fn invalid_structural_limits_return_typed_errors() {
    let ceiling = NonZeroU16::new(3).unwrap();
    assert_build_error(
        Limits {
            records: 1,
            fields: 0,
            bytes: 8,
            loss_ceiling: ceiling,
        },
        BuildError::ZeroFields,
    );
    assert_build_error(
        Limits {
            records: 1,
            fields: 4,
            bytes: 0,
            loss_ceiling: ceiling,
        },
        BuildError::ZeroBytes,
    );
    assert_build_error(
        Limits {
            records: usize::MAX,
            fields: 4,
            bytes: 8,
            loss_ceiling: ceiling,
        },
        BuildError::SizeOverflow,
    );
    assert_build_error(
        Limits {
            records: isize::MAX as usize,
            fields: 2,
            bytes: 1,
            loss_ceiling: ceiling,
        },
        BuildError::SizeOverflow,
    );
    assert_build_error(
        Limits {
            records: isize::MAX as usize,
            fields: 1,
            bytes: 2,
            loss_ceiling: ceiling,
        },
        BuildError::SizeOverflow,
    );
    assert!(Capture::new(Limits {
        records: 0,
        fields: 1,
        bytes: 1,
        loss_ceiling: ceiling,
    })
    .is_ok());
}
