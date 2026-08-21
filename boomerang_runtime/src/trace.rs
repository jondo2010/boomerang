//! Runtime ingress records deliberately carry no source provenance. A later Layer may promote a
//! neutral ingress to a propagation receive/edge only when the structured `(destination enclave,
//! action, complete tag)` candidate is unique. If multiple sends match, the Layer must retain the
//! ambiguity and must not create an exact causal edge.

/// Stable tracing target used by runtime instrumentation.
pub const TRACE_TARGET: &str = "boomerang::trace";

pub(crate) fn enabled() -> bool {
    tracing::enabled!(target: TRACE_TARGET, tracing::Level::TRACE)
}

#[inline]
pub(crate) fn collect_if_enabled<T>(collect: impl FnOnce() -> T) -> Option<T> {
    enabled().then(collect)
}

#[doc(hidden)]
pub fn logical_ns(tag: crate::Tag) -> u64 {
    let nanoseconds = tag.offset().whole_nanoseconds();
    if nanoseconds.is_negative() {
        0
    } else {
        u64::try_from(nanoseconds).unwrap_or(u64::MAX)
    }
}

#[doc(hidden)]
pub fn microstep(tag: crate::Tag) -> u64 {
    u64::try_from(tag.microstep()).expect("tag microstep does not fit in u64")
}

/// Stable event names emitted by runtime instrumentation.
pub mod event {
    pub const SCHEDULER_THREAD: &str = "scheduler_thread";
    pub const ASYNC_INGRESS: &str = "async_ingress";
    pub const TAG_PROCESS: &str = "tag_process";
    pub const REACTION_EXECUTE: &str = "reaction_execute";
    pub const ACTION_SCHEDULE: &str = "action_schedule";
    /// A reaction mutably accessed an output port through `OutputRef`.
    ///
    /// Rust's `DerefMut` boundary observes mutable access, not whether the caller subsequently
    /// assigned a different value.
    pub const PORT_WRITE: &str = "port_write";
    pub const PROPAGATION_SEND: &str = "propagation_send";
    pub const PROPAGATION_RECEIVE: &str = "propagation_receive";
    pub const FRONTIER_PUBLISH: &str = "frontier_publish";
    pub const COORDINATION_WAIT: &str = "coordination_wait";
    pub const COORDINATION_GRANT: &str = "coordination_grant";
    pub const TAG_RELEASE: &str = "tag_release";
    pub const TAG_COMPLETE: &str = "tag_complete";
    pub const CAUSAL_LINK: &str = "causal_link";
    pub const SHUTDOWN: &str = "shutdown";
    pub const DIAGNOSTIC: &str = "diagnostic";

    /// All stable event names in canonical order.
    pub const ALL: &[&str] = &[
        SCHEDULER_THREAD,
        ASYNC_INGRESS,
        TAG_PROCESS,
        REACTION_EXECUTE,
        ACTION_SCHEDULE,
        PORT_WRITE,
        PROPAGATION_SEND,
        PROPAGATION_RECEIVE,
        FRONTIER_PUBLISH,
        COORDINATION_WAIT,
        COORDINATION_GRANT,
        TAG_RELEASE,
        TAG_COMPLETE,
        CAUSAL_LINK,
        SHUTDOWN,
        DIAGNOSTIC,
    ];
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        collections::HashMap,
        fmt,
        sync::{Arc, Mutex},
    };

    use tracing::{field::Visit, span::Attributes, span::Record, Event, Id, Subscriber};
    use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

    use crate::{
        env::tests::create_enclave_pair, execute_enclaves, ActionKey, AsyncEvent, CommonContext,
        Config, Duration, EnclaveKey, Tag,
    };

    use super::{event, TRACE_TARGET};

    #[derive(Clone, Debug, Default)]
    struct CapturedRecord {
        fields: HashMap<String, String>,
        parent: Option<u64>,
    }

    #[derive(Clone, Default)]
    struct CaptureLayer {
        spans: Arc<Mutex<HashMap<u64, CapturedRecord>>>,
        events: Arc<Mutex<Vec<CapturedRecord>>>,
        lifecycle: Arc<Mutex<Vec<LifecycleRecord>>>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum LifecycleKind {
        Enter,
        Exit,
        Close,
    }

    #[derive(Clone, Copy, Debug)]
    struct LifecycleRecord {
        span: u64,
        kind: LifecycleKind,
    }

    #[derive(Default)]
    struct FieldVisitor(HashMap<String, String>);

    impl Visit for FieldVisitor {
        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            if attrs.metadata().target() != TRACE_TARGET {
                return;
            }
            let mut visitor = FieldVisitor::default();
            attrs.record(&mut visitor);
            self.spans.lock().unwrap().insert(
                id.into_u64(),
                CapturedRecord {
                    fields: visitor.0,
                    parent: attrs
                        .parent()
                        .map(Id::into_u64)
                        .or_else(|| ctx.current_span().id().map(Id::into_u64)),
                },
            );
        }

        fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
            if event.metadata().target() != TRACE_TARGET {
                return;
            }
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            let parent = event
                .parent()
                .map(Id::into_u64)
                .or_else(|| ctx.current_span().id().map(Id::into_u64));
            self.events.lock().unwrap().push(CapturedRecord {
                fields: visitor.0,
                parent,
            });
        }

        fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
            let mut visitor = FieldVisitor::default();
            values.record(&mut visitor);
            if let Some(span) = self.spans.lock().unwrap().get_mut(&id.into_u64()) {
                span.fields.extend(visitor.0);
            }
        }

        fn on_enter(&self, id: &Id, _ctx: Context<'_, S>) {
            self.record_lifecycle(id, LifecycleKind::Enter);
        }

        fn on_exit(&self, id: &Id, _ctx: Context<'_, S>) {
            self.record_lifecycle(id, LifecycleKind::Exit);
        }

        fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
            self.record_lifecycle(&id, LifecycleKind::Close);
        }
    }

    impl CaptureLayer {
        fn record_lifecycle(&self, id: &Id, kind: LifecycleKind) {
            let span = id.into_u64();
            if self.spans.lock().unwrap().contains_key(&span) {
                self.lifecycle
                    .lock()
                    .unwrap()
                    .push(LifecycleRecord { span, kind });
            }
        }
    }

    impl CapturedRecord {
        fn field(&self, name: &str) -> &str {
            self.fields
                .get(name)
                .unwrap_or_else(|| panic!("missing {name} in {:?}", self.fields))
        }
    }

    fn execute_pair_with_capture(
        configure: impl FnOnce(&mut tinymap::TinyMap<EnclaveKey, crate::Enclave>),
    ) -> (
        CaptureLayer,
        tinymap::TinySecondaryMap<EnclaveKey, crate::Env>,
    ) {
        let capture = CaptureLayer::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        let mut enclaves = create_enclave_pair();
        configure(&mut enclaves);
        let config = Config::default()
            .with_fast_forward(true)
            .with_keep_alive(false)
            .with_timeout(Duration::seconds(3));
        let envs = tracing::subscriber::with_default(subscriber, || {
            execute_enclaves(enclaves.into_iter(), config).expect("enclave execution succeeds")
        });
        (capture, envs)
    }

    fn assert_parent_reaction(
        record: &CapturedRecord,
        expected_reaction: &str,
        spans: &HashMap<u64, CapturedRecord>,
    ) {
        let parent = spans
            .get(&record.parent.expect("structured fact has a parent span"))
            .expect("parent span was captured");
        assert_eq!(parent.field("event"), event::REACTION_EXECUTE);
        assert_eq!(parent.field("reaction"), expected_reaction);
    }

    #[test]
    fn trace_target_is_stable() {
        assert_eq!(TRACE_TARGET, "boomerang::trace");
    }

    #[test]
    fn disabled_trace_target_skips_metadata_collection() {
        let metadata_collected = Cell::new(false);
        let envs =
            tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
                assert!(super::collect_if_enabled(|| metadata_collected.set(true)).is_none());
                execute_enclaves(
                    create_enclave_pair().into_iter(),
                    Config::default()
                        .with_fast_forward(true)
                        .with_keep_alive(false)
                        .with_timeout(Duration::seconds(3)),
                )
                .expect("enclave execution succeeds without tracing")
            });
        assert!(!metadata_collected.get());
        let reactor_b = envs
            .values()
            .find_map(|env| env.find_reactor_by_name("reactorB"))
            .expect("reactorB is returned");
        assert_eq!(reactor_b.get_state::<bool>(), Some(&true));
    }

    #[test]
    fn enclave_pair_emits_stateless_facts_with_reaction_parentage() {
        let (capture, envs) = execute_pair_with_capture(|_| {});

        let reactor_b = envs
            .values()
            .find_map(|env| env.find_reactor_by_name("reactorB"))
            .expect("reactorB is returned");
        assert_eq!(reactor_b.get_state::<bool>(), Some(&true));

        let spans = capture.spans.lock().unwrap().clone();
        let records = capture.events.lock().unwrap().clone();
        let lifecycle = capture.lifecycle.lock().unwrap().clone();
        for span_event in [
            event::TAG_PROCESS,
            event::REACTION_EXECUTE,
            event::COORDINATION_WAIT,
        ] {
            assert!(
                spans.values().any(|span| {
                    span.field("event") == span_event
                        && span.fields.contains_key("enclave")
                        && span.fields.contains_key("logical_ns")
                        && span.fields.contains_key("microstep")
                }),
                "missing complete {span_event} span; got {spans:?}"
            );
        }

        for (&reaction_id, reaction) in spans
            .iter()
            .filter(|(_, span)| span.field("event") == event::REACTION_EXECUTE)
        {
            let parent_id = reaction.parent.expect("reaction span has a tag parent");
            let parent = spans.get(&parent_id).expect("tag parent was captured");
            assert_eq!(parent.field("event"), event::TAG_PROCESS);
            assert_eq!(parent.field("logical_ns"), reaction.field("logical_ns"));
            assert_eq!(parent.field("microstep"), reaction.field("microstep"));

            let position = |kind| {
                lifecycle
                    .iter()
                    .position(|record| record.span == reaction_id && record.kind == kind)
                    .unwrap_or_else(|| panic!("missing {kind:?} for reaction span {reaction_id}"))
            };
            let enter = position(LifecycleKind::Enter);
            let exit = position(LifecycleKind::Exit);
            let close = position(LifecycleKind::Close);
            let tag_close = lifecycle
                .iter()
                .position(|record| record.span == parent_id && record.kind == LifecycleKind::Close)
                .expect("tag span closes");
            assert!(enter < exit && exit < close && close < tag_close);
        }

        for fact in [
            event::ACTION_SCHEDULE,
            event::PORT_WRITE,
            event::ASYNC_INGRESS,
            event::FRONTIER_PUBLISH,
            event::COORDINATION_GRANT,
            event::TAG_RELEASE,
            event::TAG_COMPLETE,
            event::SHUTDOWN,
        ] {
            assert!(
                records.iter().any(|record| record.field("event") == fact),
                "missing structured fact {fact}"
            );
        }
        assert!(!records
            .iter()
            .any(|record| record.field("event") == event::PROPAGATION_RECEIVE));

        let scheduled = records
            .iter()
            .find(|record| {
                record.field("event") == event::ACTION_SCHEDULE
                    && record.field("action") == "followup"
            })
            .expect("reaction-scheduled followup action");
        assert_parent_reaction(scheduled, "startup", &spans);

        let port_write = records
            .iter()
            .find(|record| {
                record.field("event") == event::PORT_WRITE && record.field("port") == "portA"
            })
            .expect("portA write");
        assert_parent_reaction(port_write, "startup", &spans);

        let send = spans
            .values()
            .find(|record| record.field("event") == event::PROPAGATION_SEND)
            .expect("in-process propagation send");
        assert_parent_reaction(send, "reactionA", &spans);
        assert_eq!(send.field("outcome"), "accepted");

        for record in &records {
            if matches!(
                record.field("event"),
                event::ACTION_SCHEDULE
                    | event::PORT_WRITE
                    | event::PROPAGATION_SEND
                    | event::ASYNC_INGRESS
                    | event::TAG_RELEASE
                    | event::TAG_COMPLETE
            ) {
                assert!(record.fields.contains_key("outcome"));
            }
        }
    }

    #[test]
    fn direct_logical_and_physical_events_remain_neutral_ingress() {
        let direct_logical_tag = Tag::new(Duration::nanoseconds(2), 0);
        let (capture, _) = execute_pair_with_capture(|enclaves| {
            let sender = EnclaveKey::from(1);
            let send_context = enclaves[sender].create_send_context(sender);
            assert!(send_context.schedule_external(AsyncEvent::Logical {
                tag: direct_logical_tag,
                key: ActionKey::from(0),
                value: Box::new(()),
            }));
            assert!(send_context.schedule_external(AsyncEvent::Physical {
                time: std::time::Instant::now() + std::time::Duration::from_millis(1),
                key: ActionKey::from(0),
                value: Box::new(()),
            }));
        });
        let records = capture.events.lock().unwrap().clone();

        assert!(records.iter().any(|record| {
            record.field("event") == event::ASYNC_INGRESS
                && record.field("kind") == "logical"
                && record.field("action") == "followup"
                && record.field("logical_ns") == "2"
                && record.field("outcome") == "accepted"
        }));
        assert!(records.iter().any(|record| {
            record.field("event") == event::ASYNC_INGRESS
                && record.field("kind") == "physical"
                && record.field("action") == "followup"
                && record.field("outcome") == "accepted"
        }));
        assert!(!records
            .iter()
            .any(|record| record.field("event") == event::PROPAGATION_RECEIVE));
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn scoped_subscriber_observes_parallel_reaction_children() {
        let (capture, _) = execute_pair_with_capture(|_| {});
        let spans = capture.spans.lock().unwrap().clone();
        let records = capture.events.lock().unwrap().clone();

        for (fact, reaction) in [
            (event::PORT_WRITE, "startup"),
            (event::PROPAGATION_SEND, "reactionA"),
        ] {
            let record = records
                .iter()
                .chain(spans.values())
                .find(|record| record.field("event") == fact)
                .unwrap_or_else(|| panic!("missing parallel fact {fact}"));
            assert_parent_reaction(record, reaction, &spans);
        }
    }
}
