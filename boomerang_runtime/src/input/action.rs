//! Typed compiled actions for source-owned admission.

use crate::input::{CompiledPhysicalAction, InputSourceHandle};
use crate::{Duration, ReactorData};
use std::marker::PhantomData;

/// A typed action owned by one input source in one execution.
///
/// Use this handle with [`super::InputValue`] and [`InputSourceHandle::submit`]
/// to preserve acquisition timestamps. Source admission rejects late observations
/// instead of adjusting their tags. Both host monotonic and manual clocks support it.
/// Generated initializers supply this handle for actions declared with a source.
///
/// ```
/// use boomerang_runtime::input::{InputValue, SourceAction};
///
/// fn sample(action: &SourceAction<u32>, value: u32) -> InputValue {
///     InputValue::new(action, value)
/// }
/// ```
///
/// Source ownership does not grant ordinary asynchronous sending. This handle
/// does not implement [`crate::ActionCommon`].
/// Use a separate action for ordinary event delivery through [`crate::SendContext`].
///
/// ```compile_fail,E0277
/// use boomerang_runtime::{input::SourceAction, SendContext};
///
/// fn bypass_source(send: &SendContext, action: &SourceAction<u32>) {
///     let _ = send.prepare_action(action, 42, None);
/// }
/// ```
pub struct SourceAction<T: ReactorData> {
    /// The compiled action and its destination Enclave.
    pub(crate) reference: CompiledPhysicalAction,
    /// The source and admission service for this execution.
    pub(crate) source: InputSourceHandle,
    /// The minimum delay of the bound action.
    pub(crate) delay: Duration,
    /// The payload type accepted by the bound action.
    pub(crate) payload: PhantomData<fn() -> T>,
}
impl<T: ReactorData> Clone for SourceAction<T> {
    fn clone(&self) -> Self {
        Self {
            reference: self.reference,
            source: self.source.clone(),
            delay: self.delay,
            payload: PhantomData,
        }
    }
}
impl<T: ReactorData> SourceAction<T> {
    /// Get metadata for admission, including the payload type and execution identity.
    pub(crate) fn admission(&self) -> AdmittedAction {
        AdmittedAction {
            reference: self.reference,
            source: self.source.clone(),
            delay: self.delay,
            payload: std::any::TypeId::of::<T>(),
        }
    }
}
/// Action metadata carried with one uniquely owned admitted payload.
pub(crate) struct AdmittedAction {
    /// The compiled action and its destination Enclave.
    pub(crate) reference: CompiledPhysicalAction,
    /// Bound source and execution identity.
    pub(crate) source: InputSourceHandle,
    /// The minimum delay of the bound action.
    pub(crate) delay: crate::Duration,
    /// Concrete action payload type.
    pub(crate) payload: std::any::TypeId,
}
