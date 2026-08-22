//! Rerun trace visualization adapter for Boomerang.

#![deny(missing_docs)]

mod entities;
mod layer;
mod session;

pub use session::{RerunSession, RerunSessionBuildError, RerunSessionFinishError};
