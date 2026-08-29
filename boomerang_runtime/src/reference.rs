//! Synchronous host execution for validated compiled enclave images.

use tinymap::TinyMap;

use crate::{
    image::{EnclaveImage, ImageValidationError, StateSlotIndex},
    run_owned_scheduler,
    storage::owned::StoredState,
    Config, OwnedBindings, OwnedStorage, OwnedStorageError, ReactorData, RuntimeError, Tag,
};

/// Failure while validating, initializing, or synchronously executing a compiled image.
#[derive(Debug, thiserror::Error)]
pub enum ExecuteOwnedError<'image> {
    /// The borrowed compiled image was structurally invalid.
    #[error("invalid compiled image: {0}")]
    ImageValidation(ImageValidationError<'image>),
    /// Direct bindings could not initialize the owned storage described by the image.
    #[error("invalid direct bindings or owned storage: {0}")]
    Storage(#[source] OwnedStorageError),
    /// The scheduler's local logical-time coordination failed.
    #[error("compiled scheduler coordination failed: {0}")]
    SchedulerRuntime(#[source] RuntimeError),
    /// A directly bound reaction failed while the scheduler was executing.
    #[error("compiled scheduler execution failed: {0}")]
    SchedulerExecution(#[source] OwnedStorageError),
}

/// A typed state-access failure from an owned compiled-image execution result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateAccessError {
    /// The requested slot exceeds the dense state table returned by execution.
    #[error("state slot {slot} is out of range")]
    OutOfRange {
        /// The requested compiled state storage slot.
        slot: StateSlotIndex,
    },
    /// The requested concrete Rust type differs from the state value's recorded type.
    #[error("state slot {slot} has type {found}, not {expected}")]
    TypeMismatch {
        /// The requested compiled state storage slot.
        slot: StateSlotIndex,
        /// The requested concrete Rust type.
        expected: &'static str,
        /// The concrete Rust type captured when the state was initialized.
        found: &'static str,
    },
}

/// Final owned state retained after a synchronous compiled-image execution.
///
/// This result owns no scheduler machinery or compiled-image borrow and can therefore outlive the
/// `EnclaveImage` passed to [`execute_owned`].
pub struct OwnedExecutionResult {
    states: TinyMap<StateSlotIndex, StoredState>,
    final_tag: Tag,
}

impl OwnedExecutionResult {
    /// Borrows the state stored at `slot` as its original concrete type.
    ///
    /// The state is read-only. Out-of-range and mismatched-type lookups return a typed
    /// [`StateAccessError`].
    pub fn state<T: ReactorData>(&self, slot: StateSlotIndex) -> Result<&T, StateAccessError> {
        if slot.as_u32() as usize >= self.states.len() {
            return Err(StateAccessError::OutOfRange { slot });
        }
        let state = &self.states[slot];
        state
            .value
            .downcast_ref::<T>()
            .ok_or(StateAccessError::TypeMismatch {
                slot,
                expected: std::any::type_name::<T>(),
                found: state.type_name,
            })
    }

    /// Returns the last logical tag at which non-shutdown work was processed.
    ///
    /// Returns [`Tag::NEVER`] if execution reached only terminal shutdown processing.
    pub const fn final_tag(&self) -> Tag {
        self.final_tag
    }
}

/// Validates and synchronously executes a borrowed compiled enclave image with direct bindings.
///
/// The image is borrowed only for execution. `bindings` is consumed to initialize one owned
/// storage instance, and the returned result retains only the final owned state and tag.
///
/// # Errors
///
/// Returns [`ExecuteOwnedError`] when image validation, binding/storage initialization, local
/// coordination, or direct reaction execution fails.
pub fn execute_owned<'image>(
    image: &'image EnclaveImage<'image>,
    bindings: OwnedBindings,
    config: Config,
) -> Result<OwnedExecutionResult, ExecuteOwnedError<'image>> {
    let image =
        crate::image::EnclaveImageView::new(image).map_err(ExecuteOwnedError::ImageValidation)?;
    let mut storage = OwnedStorage::new(image, bindings).map_err(ExecuteOwnedError::Storage)?;
    let final_tag = run_owned_scheduler(&mut storage, &config).map_err(|error| match error {
        crate::sched::SchedulerError::Coordination(source) => {
            ExecuteOwnedError::SchedulerRuntime(source)
        }
        crate::sched::SchedulerError::Execution(source) => {
            ExecuteOwnedError::SchedulerExecution(source)
        }
    })?;
    Ok(OwnedExecutionResult {
        states: storage.into_states(),
        final_tag,
    })
}
