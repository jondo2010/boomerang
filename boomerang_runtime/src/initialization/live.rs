//! Deferred state construction for the live builder/runner, before Store pinning.
use super::{InitializationError, StateInitializationError};
use crate::{
    Action, ActionKey, AsyncActionRef, BaseAction, BaseReactor, DynActionRef, Enclave, EnclaveKey,
    Reactor, ReactorData, SendContext,
};
use tinymap::TinyMap;

/// A typed requirement resolved through the builder's existing action aliases.
#[derive(Clone)]
pub struct LiveActionCapability {
    key: ActionKey,
    accepts: fn(&dyn BaseAction) -> bool,
}
impl LiveActionCapability {
    /// Retains a concrete payload check without constructing or registering an action.
    pub fn new<T: ReactorData>(key: ActionKey) -> Self {
        Self {
            key,
            accepts: |action| !action.is_logical() && action.downcast_ref::<Action<T>>().is_some(),
        }
    }
}
/// Borrowed live action storage and an already-started host monotonic clock.
pub struct LiveInitContext<'a> {
    actions: &'a TinyMap<ActionKey, Box<dyn BaseAction>>,
    requirements: &'a [LiveActionCapability],
    send: SendContext,
}
impl LiveInitContext<'_> {
    /// Binds only a declared action, preserving its actual delay and payload metadata.
    pub fn async_action<T: ReactorData>(
        &self,
        key: ActionKey,
    ) -> Result<AsyncActionRef<T>, InitializationError> {
        let action = self
            .actions
            .get(key)
            .ok_or(InitializationError::InvalidCapabilities)?;
        if !self
            .requirements
            .iter()
            .any(|required| required.key == key && (required.accepts)(action.as_ref()))
            || action.is_logical()
        {
            return Err(InitializationError::InvalidCapabilities);
        }
        AsyncActionRef::try_from(DynActionRef(action.as_ref()))
            .map_err(|_| InitializationError::InvalidCapabilities)
    }
    /// Shares the scheduler's original mailbox and live clock.
    pub fn send_context(&self) -> SendContext {
        self.send.clone()
    }
}
/// Erased construction retained until global capability preflight completes.
type Initializer = Box<
    dyn FnOnce(&LiveInitContext<'_>) -> Result<Box<dyn BaseReactor>, StateInitializationError>
        + Send
        + Sync,
>;
/// An uninitialized reactor shell. It is never exposed to reaction execution.
pub(crate) struct DeferredReactor {
    name: String,
    requirements: Vec<LiveActionCapability>,
    initialize: Option<Initializer>,
}
impl std::fmt::Debug for DeferredReactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeferredReactor")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
impl BaseReactor for DeferredReactor {
    fn name(&self) -> &str {
        &self.name
    }
}
/// Creates a deferred shell without constructing a temporary application state.
/// Builders must pass it to the normal live execution lifecycle before reactions run.
pub fn deferred_reactor<T: ReactorData>(
    name: &str,
    requirements: Vec<LiveActionCapability>,
    initialize: impl FnOnce(&LiveInitContext<'_>) -> Result<T, StateInitializationError>
        + Send
        + Sync
        + 'static,
) -> Box<dyn BaseReactor> {
    let owned_name = name.to_owned();
    Box::new(DeferredReactor {
        name: owned_name.clone(),
        requirements,
        initialize: Some(Box::new(move |context| {
            Ok(Reactor::new(&owned_name, initialize(context)?).boxed())
        })),
    })
}
/// Whether execution must bind host monotonic time before constructing user state.
pub(crate) fn has_initializers(enclave: &Enclave) -> bool {
    enclave
        .env
        .reactors
        .values()
        .any(|reactor| reactor.is::<DeferredReactor>())
}
/// Validates every live typed requirement without calling user code.
pub(crate) fn preflight(enclave: &Enclave) -> Result<(), InitializationError> {
    if !crate::store::Store::valid_layout(&enclave.env, &enclave.graph)
        || enclave
            .graph
            .reaction_reactors
            .values()
            .any(|key| enclave.graph.reactor_bank_infos.get(*key).is_none())
    {
        return Err(InitializationError::InvalidStorageLayout);
    }
    for reactor in enclave.env.reactors.values() {
        if let Some(deferred) = reactor.downcast_ref::<DeferredReactor>() {
            for requirement in &deferred.requirements {
                let action = enclave
                    .env
                    .actions
                    .get(requirement.key)
                    .ok_or(InitializationError::InvalidCapabilities)?;
                if !(requirement.accepts)(action.as_ref()) {
                    return Err(InitializationError::InvalidCapabilities);
                }
            }
        }
    }
    Ok(())
}
/// Replaces shells with their final real states before Store creates cached references.
pub(crate) fn initialize(
    enclave: &mut Enclave,
    key: EnclaveKey,
) -> Result<(), StateInitializationError> {
    let send = enclave.create_send_context(key);
    for reactor in enclave.env.reactors.values_mut() {
        if let Some(deferred) = reactor.downcast_mut::<DeferredReactor>() {
            let context = LiveInitContext {
                actions: &enclave.env.actions,
                requirements: &deferred.requirements,
                send: send.clone(),
            };
            let initialized = deferred.initialize.take().expect("initializer runs once")(&context)?;
            *reactor = initialized;
        }
    }
    Ok(())
}
/// Close mailboxes before states or worker closures can be destroyed.
/// A successful worker disarms global cancellation and closes only its own mailbox.
#[derive(Clone, Debug)]
pub(crate) struct CloseMailboxes(pub(crate) Vec<crate::Sender<crate::AsyncEvent>>);
impl CloseMailboxes {
    pub(crate) fn close(&self) {
        for sender in &self.0 {
            let _ = sender.close();
        }
    }
}
impl Drop for CloseMailboxes {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommonContext, Config};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    /// A returned state must observe closure before its destructor could join a sender.
    struct ReturnedGuard {
        send: SendContext,
        dropped: Arc<AtomicUsize>,
    }
    impl Drop for ReturnedGuard {
        fn drop(&mut self) {
            assert_eq!(
                self.send.try_shutdown(None),
                Err(crate::TryShutdownError::Closed)
            );
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn guarded_enclave(calls: Arc<AtomicUsize>, dropped: Arc<AtomicUsize>) -> Enclave {
        let mut enclave = Enclave::with_event_q_size(1);
        enclave.insert_reactor(
            deferred_reactor("guard", vec![], move |context| {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(context.send_context().try_get_physical_time().is_ok());
                Ok(ReturnedGuard {
                    send: context.send_context(),
                    dropped,
                })
            }),
            None,
        );
        enclave
    }
    #[test]
    fn invalid_later_enclave_rejects_before_any_state_effect() {
        let calls = Arc::new(AtomicUsize::new(0));
        let first = guarded_enclave(calls.clone(), Arc::new(AtomicUsize::new(0)));
        let mut invalid = Enclave::with_event_q_size(1);
        let action =
            invalid.insert_action(|key| Action::<u32>::new("sample", key, None, false).boxed());
        invalid.insert_reactor(
            deferred_reactor(
                "invalid",
                vec![LiveActionCapability::new::<u64>(action)],
                |_| Ok(()),
            ),
            None,
        );
        assert!(crate::execute_enclaves(
            [(EnclaveKey::new(0), first), (EnclaveKey::new(1), invalid)].into_iter(),
            Config::default()
        )
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn no_reaction_enclave_initializes_once_and_closes_before_state_drop() {
        let calls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let enclave = guarded_enclave(calls.clone(), dropped.clone());
        let result = crate::execute_enclaves(
            [(EnclaveKey::new(0), enclave)].into_iter(),
            Config::default(),
        )
        .unwrap();
        assert!(result.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn later_initializer_error_or_panic_closes_before_returned_state_drop() {
        for panic in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let dropped = Arc::new(AtomicUsize::new(0));
            let first = guarded_enclave(calls.clone(), dropped.clone());
            let mut later = Enclave::with_event_q_size(1);
            later.insert_reactor(
                deferred_reactor::<()>("later", vec![], move |_| {
                    if panic {
                        panic!("initializer panic");
                    }
                    Err(std::io::Error::other("initializer failure").into())
                }),
                None,
            );
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::execute_enclaves(
                    [(EnclaveKey::new(0), first), (EnclaveKey::new(1), later)].into_iter(),
                    Config::default(),
                )
            }));
            assert!(if panic {
                result.is_err()
            } else {
                result.unwrap().is_err()
            });
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn standalone_drop_closes_before_returned_state_drop() {
        let calls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let enclave = guarded_enclave(calls.clone(), dropped.clone());
        let scheduler =
            crate::Scheduler::try_new(EnclaveKey::new(0), enclave, Config::default(), None)
                .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(scheduler);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn invalid_store_layout_rejects_before_initializer_effects() {
        for bulk in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let mut enclave = Enclave::with_event_q_size(1);
            enclave.insert_reactor(
                deferred_reactor("state", vec![], move |_| {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
                None,
            );
            // No matching graph row for this reaction: Store must never consume a
            // successfully initialized state before noticing this structural error.
            enclave.env.reactions.insert(crate::Reaction::new(
                "invalid",
                crate::reaction_closure!(),
                None,
            ));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if bulk {
                    crate::execute_enclaves(
                        [(EnclaveKey::new(0), enclave)].into_iter(),
                        Config::default(),
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string())
                } else {
                    crate::Scheduler::try_new(EnclaveKey::new(0), enclave, Config::default(), None)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }
            }));
            assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "invalid storage must reject before initializer effects"
            );
            assert!(result
                .expect("fallible construction must return the structural error")
                .is_err());
        }
    }
}
