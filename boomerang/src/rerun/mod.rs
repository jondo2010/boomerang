//! Optional Rerun recording support.

mod entities;
mod layer;
pub mod schema;
mod session;

pub use entities::{
    TraceId, TraceStateChange, TraceStateRecord, TraceTimePoint, TraceWriter, TraceWriterError,
};
pub use layer::RerunLayer;
pub use schema::{TraceEvent, TraceRecord, TraceTag};
pub use session::{
    BlueprintConfig, FlushDriver, RerunSession, RerunSessionBuildError, RerunSessionBuilder,
    RerunSessionFinishError, SinkConfig, SinkConfigError,
};
