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
    /// Owned storage initialization or directly bound reaction execution failed.
    #[error("compiled storage or reaction execution failed: {0}")]
    Storage(#[from] OwnedStorageError),
    /// The scheduler's local logical-time coordination failed.
    #[error("compiled scheduler coordination failed: {0}")]
    Coordination(#[source] RuntimeError),
}

impl<'image> From<ImageValidationError<'image>> for ExecuteOwnedError<'image> {
    fn from(source: ImageValidationError<'image>) -> Self {
        Self::ImageValidation(source)
    }
}

impl<'image> From<crate::sched::SchedulerError<OwnedStorageError>> for ExecuteOwnedError<'image> {
    fn from(error: crate::sched::SchedulerError<OwnedStorageError>) -> Self {
        match error {
            crate::sched::SchedulerError::Coordination(source) => Self::Coordination(source),
            crate::sched::SchedulerError::Execution(source) => Self::Storage(source),
        }
    }
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
    /// Final owned reactor states keyed by compiled storage slot.
    states: TinyMap<StateSlotIndex, StoredState>,
    /// Last logical tag that processed non-terminal work.
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
    let image = crate::image::EnclaveImageView::new(image)?;
    let mut storage = OwnedStorage::new(image, bindings)?;
    let final_tag = run_owned_scheduler(&mut storage, &config)?;
    Ok(OwnedExecutionResult {
        states: storage.into_states(),
        final_tag,
    })
}
