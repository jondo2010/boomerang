use crate::{BoundedSubscriber, Config, Value};

#[test]
fn native_scopes_updates_and_reclamation_keep_records_self_contained() {
    let (subscriber, handle) = BoundedSubscriber::new(Config::default()).unwrap();
    let _producer = handle.prepare_current_thread().unwrap();
    tracing::subscriber::with_default(subscriber, || {
        let parent = tracing::info_span!("parent", key = 1u64);
        let child = tracing::info_span!(parent: &parent, "child", key = 2u64);
        drop(parent);
        child.in_scope(|| tracing::info!(value = 3u64));
        child.record("key", 4u64);
        child.in_scope(|| tracing::info!(value = 5u64));
        drop(child);
        tracing::info!(parent: None, value = 6u64);
    });
    let records = handle.snapshot();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].scopes.len(), 2);
    assert_eq!(records[0].scopes[0].metadata.name(), "parent");
    assert_eq!(records[0].scopes[0].fields[0].value, Value::U64(1));
    assert_eq!(records[0].scopes[1].fields[0].value, Value::U64(2));
    assert_eq!(records[1].scopes[1].fields[0].value, Value::U64(4));
    assert!(records[2].scopes.is_empty());
    assert_eq!(handle.loss().invalid_context.value, 0);
}

#[test]
fn unprepared_threads_and_exhausted_spans_never_become_root() {
    let (subscriber, handle) = BoundedSubscriber::new(Config {
        spans: 1,
        ..Config::default()
    })
    .unwrap();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(parent: None, value = 0u64);
        let _producer = handle.prepare_current_thread().unwrap();
        let kept = tracing::info_span!("kept");
        let lost = tracing::info_span!("lost");
        lost.in_scope(|| tracing::info!(value = 1u64));
        drop(lost);
        kept.in_scope(|| tracing::info!(value = 2u64));
    });
    assert_eq!(handle.snapshot().len(), 1);
    assert_eq!(handle.snapshot()[0].scopes[0].metadata.name(), "kept");
    assert_eq!(handle.loss().invalid_context.value, 2);
    assert_eq!(handle.lifecycle_loss().span_admission.value, 1);
}

#[test]
fn cloned_span_can_move_between_prepared_threads_and_outlive_its_creator() {
    let (subscriber, handle) = BoundedSubscriber::new(Config::default()).unwrap();
    let dispatch = tracing::Dispatch::new(subscriber);
    let span = {
        let _producer = handle.prepare_current_thread().unwrap();
        tracing::dispatcher::with_default(&dispatch, || tracing::info_span!("moved", key = 7u64))
    };
    let other = span.clone();
    drop(span);
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _producer = handle.prepare_current_thread().unwrap();
                tracing::dispatcher::with_default(&dispatch, || {
                    other.in_scope(|| tracing::info!(value = 8u64));
                });
            })
            .join()
            .unwrap();
    });
    let record = handle.snapshot().pop().unwrap();
    assert_eq!(record.scopes[0].fields[0].value, Value::U64(7));
    assert_eq!(record.fields[0].value, Value::U64(8));
}

#[test]
fn repeated_span_reuse_and_updates_do_not_consume_capacity() {
    let (subscriber, handle) = BoundedSubscriber::new(Config {
        spans: 1,
        span_bytes: 2,
        ..Config::default()
    })
    .unwrap();
    let _producer = handle.prepare_current_thread().unwrap();
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..20 {
            let span = tracing::info_span!("reused", text = "ab");
            for _ in 0..20 {
                span.record("text", "cd");
            }
            span.in_scope(|| tracing::info!(ok = true));
        }
    });
    assert_eq!(handle.lifecycle_loss().span_admission.value, 0);
    assert_eq!(handle.lifecycle_loss().span_update.value, 0);
    assert_eq!(
        handle.snapshot().last().unwrap().scopes[0].fields[0].value,
        Value::Str("cd".into())
    );
}

#[test]
fn invalid_context_is_visible_and_recovers_only_at_a_known_boundary() {
    let (subscriber, handle) = BoundedSubscriber::new(Config {
        spans: 1,
        depth: 1,
        ..Config::default()
    })
    .unwrap();
    tracing::subscriber::with_default(subscriber, || {
        let producer = handle.prepare_current_thread().unwrap();
        let good = tracing::info_span!("good");
        let lost = tracing::info_span!("lost");
        lost.in_scope(|| {
            assert!(!handle.current_thread_is_valid());
            let propagated = tracing::Span::current();
            tracing::info!(parent: &propagated, value = 0u64);
        });
        assert!(handle.current_thread_is_valid());
        let first = good.enter();
        let overflow = good.enter();
        tracing::info!(parent: None, value = 1u64);
        drop(overflow);
        drop(first);
        assert!(!handle.current_thread_is_valid());
        drop(producer);
        let _producer = handle.prepare_current_thread().unwrap();
        good.in_scope(|| tracing::info!(value = 2u64));
    });
    assert_eq!(handle.snapshot().len(), 1);
    assert_eq!(handle.loss().invalid_context.value, 2);
    assert_eq!(handle.lifecycle_loss().invalid_context_transitions.value, 1);
}

#[test]
fn sibling_poll_scopes_and_failed_updates_cannot_leak_context() {
    let (subscriber, handle) = BoundedSubscriber::new(Config::default()).unwrap();
    let _producer = handle.prepare_current_thread().unwrap();
    tracing::subscriber::with_default(subscriber, || {
        let a = tracing::info_span!("a", key = 1u64);
        let b = tracing::info_span!("b", key = 2u64);
        for span in [&a, &b, &a, &b] {
            span.in_scope(|| tracing::info!(ok = true));
        }
        a.record("key", tracing::field::debug(&"unsupported"));
        a.in_scope(|| tracing::info!(ok = false));
        b.in_scope(|| tracing::info!(ok = true));
    });
    let names: Vec<_> = handle
        .snapshot()
        .iter()
        .map(|r| r.scopes[0].metadata.name())
        .collect();
    assert_eq!(names, ["a", "b", "a", "b", "b"]);
    assert_eq!(handle.lifecycle_loss().span_update.value, 1);
    assert_eq!(handle.loss().invalid_context.value, 1);
}

#[test]
fn producer_and_reference_exhaustion_are_visible_without_corrupting_live_spans() {
    use std::num::NonZeroU16;
    let (subscriber, handle) = BoundedSubscriber::new(Config {
        producers: 1,
        references: NonZeroU16::new(2).unwrap(),
        ..Config::default()
    })
    .unwrap();
    let _producer = handle.prepare_current_thread().unwrap();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                assert!(matches!(
                    handle.prepare_current_thread(),
                    Err(crate::PrepareError::Capacity)
                ))
            })
            .join()
            .unwrap();
    });
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("span");
        let copy = span.clone();
        let failed = span.clone();
        failed.in_scope(|| tracing::info!(value = 0u64));
        drop(copy);
        span.in_scope(|| tracing::info!(value = 1u64));
        let other = tracing::info_span!("other");
        span.follows_from(&other);
    });
    assert_eq!(handle.snapshot().len(), 1);
    assert_eq!(handle.loss().invalid_context.value, 1);
    assert_eq!(handle.lifecycle_loss().span_admission.value, 1);
    assert_eq!(handle.lifecycle_loss().producer_admission.value, 1);
    assert_eq!(
        handle.lifecycle_loss().unsupported_context_operation.value,
        1
    );
}

#[test]
fn manual_native_filtered_values_do_not_capture_format_or_count_loss() {
    use tracing_core::{metadata::Kind, span::Attributes, Callsite, Event, Level};
    struct Bomb;
    impl std::fmt::Debug for Bomb {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("filtered formatter");
        }
    }
    let (subscriber, handle) = BoundedSubscriber::new(Config {
        targets: &["kept"],
        ..Config::default()
    })
    .unwrap();
    let _producer = handle.prepare_current_thread().unwrap();
    let dispatch = tracing::Dispatch::new(subscriber);
    tracing::dispatcher::with_default(&dispatch, || {
        let callsite = tracing::callsite! { name: "filtered", kind: Kind::EVENT, target: "excluded", level: Level::INFO, fields: value };
        let value = tracing::field::debug(Bomb);
        let values: [Option<&dyn tracing_core::field::Value>; 1] = [Some(&value)];
        let fields = callsite.metadata().fields().value_set_all(&values);
        Event::child_of(None, callsite.metadata(), &fields);
        let callsite = tracing::callsite! { name: "filtered span", kind: Kind::SPAN, target: "excluded", level: Level::INFO, fields: value };
        let fields = callsite.metadata().fields().value_set_all(&values);
        let id = dispatch.new_span(&Attributes::new_root(callsite.metadata(), &fields));
        let copy = dispatch.clone_span(&id);
        dispatch.enter(&id);
        tracing::info!(target: "kept", ok = true);
        dispatch.exit(&id);
        dispatch.try_close(copy);
        dispatch.try_close(id);
    });
    let records = handle.snapshot();
    assert_eq!(records.len(), 1);
    assert!(records[0].scopes.is_empty());
    assert_eq!(handle.loss(), crate::LossSnapshot::default());
    assert_eq!(handle.lifecycle_loss(), crate::LifecycleLoss::default());
}

#[test]
fn invalid_ancestor_precedes_excess_descendant_fields() {
    let (subscriber, handle) = BoundedSubscriber::new(Config {
        fields: 1,
        ..Config::default()
    })
    .unwrap();
    let _producer = handle.prepare_current_thread().unwrap();
    tracing::subscriber::with_default(subscriber, || {
        let parent = tracing::info_span!("parent", value = 1u64);
        let child = tracing::info_span!(parent: &parent, "child", a = 1u64, b = 2u64);
        parent.record("value", tracing::field::debug("unsupported"));
        child.in_scope(|| tracing::info!(value = 3u64));
    });
    assert!(handle.snapshot().is_empty());
    assert_eq!(handle.loss().invalid_context.value, 1);
    assert_eq!(handle.loss().field_limit.value, 0);
}

#[test]
#[cfg_attr(miri, ignore = "run alone under Miri with --ignored --exact")]
fn first_native_span_lifecycle_does_not_allocate() {
    use crate::test_allocation as allocation;
    const CHILD: &str = "TRACING_BOUNDED_SUBSCRIBER_CHILD";
    const COMPLETE: &str = "native subscriber probe complete";
    if !cfg!(miri) && std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "subscriber_tests::first_native_span_lifecycle_does_not_allocate",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(COMPLETE));
        return;
    }
    let (subscriber, handle) = BoundedSubscriber::new(Config::default()).unwrap();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    let _producer = handle.prepare_current_thread().unwrap();
    allocation::prepare();
    let control = allocation::measure(|| drop(std::hint::black_box(Box::new(42u64))));
    assert!(control.allocations > 0 && control.deallocations > 0);
    let counts = allocation::measure(|| {
        let parent = tracing::info_span!("parent", key = 1u64);
        let child = tracing::info_span!(parent: &parent, "child", key = 2u64);
        let copy = child.clone();
        copy.record("key", 3u64);
        copy.in_scope(|| {
            let current = tracing::Span::current();
            tracing::info!(parent: &current, value = 4u64);
        });
        drop(parent);
        drop(child);
        drop(copy);
    });
    assert_eq!(counts, allocation::Counts::default());
    let records = handle.snapshot();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].scopes.len(), 2);
    assert_eq!(records[0].scopes[1].fields[0].value, Value::U64(3));
    assert_eq!(handle.loss(), crate::LossSnapshot::default());
    assert_eq!(handle.lifecycle_loss(), crate::LifecycleLoss::default());
    println!("{COMPLETE}");
}
