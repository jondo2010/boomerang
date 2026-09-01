//! Standard-library reference implementation for synchronously executing validated compiled enclave images as a behavioral baseline for target executors.

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
    /// The image contains scheduler-boundary routes, which this local executor cannot deliver.
    #[error("compiled reference execution does not support {count} scheduler-boundary route(s)")]
    RoutesUnsupported {
        /// Number of routes present in the validated enclave image.
        count: usize,
    },
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
/// It owns no scheduler machinery or image borrow and may outlive the executed image.
pub struct OwnedExecutionResult {
    /// Final owned reactor states keyed by compiled storage slot.
    states: TinyMap<StateSlotIndex, StoredState>,
    /// Last logical tag that processed non-terminal work.
    final_tag: Tag,
}

impl OwnedExecutionResult {
    /// Borrows the state stored at `slot` as its original concrete type.
    /// Invalid slots or concrete types return [`StateAccessError`].
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
    /// Returns [`Tag::NEVER`] if execution reached only terminal shutdown processing.
    pub const fn final_tag(&self) -> Tag {
        self.final_tag
    }
}

/// Validates and synchronously executes a borrowed compiled enclave image with direct bindings.
/// Consumes `bindings`; the result retains only final owned state and the last work tag.
///
/// # Errors
///
/// Returns [`ExecuteOwnedError`] for validation, storage, coordination, or reaction failures.
pub fn execute_owned<'image>(
    image: &EnclaveImage<'image>,
    bindings: OwnedBindings,
    config: Config,
) -> Result<OwnedExecutionResult, ExecuteOwnedError<'image>> {
    let image = crate::image::EnclaveImageView::new(image)?;
    let unsupported_routes = image.routes().len();
    if unsupported_routes != 0 {
        return Err(ExecuteOwnedError::RoutesUnsupported {
            count: unsupported_routes,
        });
    }
    let mut storage = OwnedStorage::new(image, bindings)?;
    let final_tag = run_owned_scheduler(&mut storage, &config)?;
    Ok(OwnedExecutionResult {
        states: storage.into_states(),
        final_tag,
    })
}
