#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(unsafe_code)]
#![deny(clippy::all)]

#[cfg(all(feature = "keyboard", not(windows)))]
pub mod keyboard_events;
#[cfg(feature = "replay")]
pub mod replay;
#[cfg(feature = "runner")]
pub mod runner;
pub mod timeout;
