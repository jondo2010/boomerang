//! Optional Rerun recording support.

mod entities;
mod layer;
pub mod schema;
mod session;

pub use entities::{
    TraceId, TraceStateChange, TraceStateRecord, TraceTimePoint, TraceWriter, TraceWriterError,
};
// Temporary compatibility bridge for the dynamic writer and layer. Task 2 replaces this with the
// canonical `schema::TraceRecord` throughout those modules.
#[doc(hidden)]
pub use entities::TraceRecord;
pub use layer::RerunLayer;
pub use schema::{TraceEvent, TraceRecord as TypedTraceRecord, TraceTag};
pub use session::{
    BlueprintConfig, FlushDriver, RerunSession, RerunSessionBuildError, RerunSessionBuilder,
    RerunSessionFinishError, SinkConfig, SinkConfigError,
};
