//! Initialization capabilities backed by compiled action rows and prepared owned storage.

use super::InitializationError;
#[cfg(feature = "hosted-source-input")]
use crate::image::EnclaveIndex;
use crate::{storage::owned::OwnedStorage, PayloadType, ReactorData};
use std::any::TypeId;

/// A typed payload expectation attached to an existing compiled physical action.
#[derive(Clone, Debug)]
pub struct ActionCapability {
    /// Existing action row; no independent target registry is constructed.
    pub(crate) action: crate::image::ActionIndex,
    /// Type validated against the actual factory before application state construction.
    pub(crate) payload: TypeId,
}
impl ActionCapability {
    /// Records a compiler-resolved action row and its statically selected payload.
    pub fn new<T: ReactorData>(action: crate::image::ActionIndex, _: PayloadType<T>) -> Self {
        Self {
            action,
            payload: TypeId::of::<T>(),
        }
    }
}
/// Compiler-resolved requirements for one component state initializer.
#[derive(Clone, Debug, Default)]
pub struct StateCapabilities {
    /// Source owned by this component within the Federate's dense source domain.
    #[cfg(feature = "hosted-source-input")]
    pub source: Option<crate::input::SourceKey>,
    /// Existing physical action rows whose typed handles the initializer receives.
    pub actions: Vec<ActionCapability>,
    /// Whether this initializer owns the selected manual clock's write capability.
    pub clock_control: bool,
}
/// Borrowed initialized endpoints used by generated named capability construction.
/// All requirements are validated before any component initializer is invoked.
pub struct StateInitContext<'a, 'image> {
    /// Actual prepared action and context storage.
    pub(crate) storage: &'a OwnedStorage<'image>,
    /// Original compiled Enclave identity.
    #[cfg(feature = "hosted-source-input")]
    pub(crate) enclave: EnclaveIndex,
    /// Requirements belonging to this state binding.
    pub(crate) requirements: &'a StateCapabilities,
    /// Shared input service, independent of clock selection.
    #[cfg(feature = "hosted-source-input")]
    pub(crate) inputs: Option<&'a crate::input::InputAdmission>,
    /// Optional selected write capability, supplied only to its declared owner.
    #[cfg(feature = "hosted-manual-clock")]
    pub(crate) clock: Option<&'a crate::clock::ManualClock>,
}
impl StateInitContext<'_, '_> {
    /// Returns this initializer's execution-bound source.
    #[cfg(feature = "hosted-source-input")]
    pub fn source(&self) -> Result<crate::input::InputSourceHandle, InitializationError> {
        self.inputs
            .ok_or(InitializationError::InvalidCapabilities)?
            .source_handle(
                self.requirements
                    .source
                    .ok_or(InitializationError::InvalidCapabilities)?,
            )
            .map_err(|_| InitializationError::InvalidCapabilities)
    }
    /// Bind a declared action to this initializer's source. Admission preserves acquisition time.
    /// The handle contains the compiled action identity and its minimum delay.
    #[cfg(feature = "hosted-source-input")]
    pub fn source_action<T: ReactorData>(
        &self,
        action: crate::image::ActionIndex,
    ) -> Result<crate::input::SourceAction<T>, InitializationError> {
        if !self
            .requirements
            .actions
            .iter()
            .any(|required| required.action == action && required.payload == TypeId::of::<T>())
        {
            return Err(InitializationError::InvalidCapabilities);
        }
        let source = self.source()?;
        let stored = self.storage.scheduler_action_ref(action);
        if !stored.is::<crate::Action<T>>() {
            return Err(InitializationError::InvalidCapabilities);
        }
        Ok(crate::input::SourceAction {
            reference: crate::input::CompiledPhysicalAction::new(self.enclave, action),
            source,
            delay: stored.min_delay().unwrap_or_default(),
            payload: std::marker::PhantomData,
        })
    }
    /// Binds an ordinary source-free physical action for `SendContext` scheduling.
    pub fn async_action<T: ReactorData>(
        &self,
        action: crate::image::ActionIndex,
    ) -> Result<crate::AsyncActionRef<T>, InitializationError> {
        if self.requirements.has_source()
            || !self
                .requirements
                .actions
                .iter()
                .any(|required| required.action == action && required.payload == TypeId::of::<T>())
        {
            return Err(InitializationError::InvalidCapabilities);
        }
        crate::AsyncActionRef::try_from(crate::DynActionRef(
            self.storage.scheduler_action_ref(action),
        ))
        .map_err(|_| InitializationError::InvalidCapabilities)
    }
    /// Shares the ordinary asynchronous action scheduling context.
    pub fn send_context(&self) -> crate::SendContext {
        self.storage.scheduler_send_context()
    }
    /// Returns manual write authority only when explicitly declared.
    #[cfg(feature = "hosted-manual-clock")]
    pub fn clock_control(&self) -> Result<crate::clock::ManualClock, InitializationError> {
        self.clock
            .filter(|_| self.requirements.clock_control)
            .cloned()
            .ok_or(InitializationError::InvalidCapabilities)
    }
}
impl StateCapabilities {
    /// Whether this initializer requests source ownership rather than ordinary sending.
    pub(crate) fn has_source(&self) -> bool {
        #[cfg(feature = "hosted-source-input")]
        {
            self.source.is_some()
        }
        #[cfg(not(feature = "hosted-source-input"))]
        {
            false
        }
    }
}
