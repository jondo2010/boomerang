//! Recording and replaying of Boomerang actions.

mod recorder;

pub use recorder::{inject_recorder, Recorder, RecorderBuilder};

use boomerang::runtime;

#[derive(serde::Serialize)]
pub struct RecordingHeader<'a> {
    name: &'a str,
    start_tag: runtime::Tag,
}

#[derive(serde::Serialize)]
struct Record<'a> {
    name: &'a str,
    key: runtime::ActionKey,
    tag: runtime::Tag,
    value: Option<&'a dyn erased_serde::Serialize>,
}
