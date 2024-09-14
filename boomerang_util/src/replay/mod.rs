#![doc=include_str!("README.md")]

mod recorder;
//mod replayer;

pub use recorder::inject_recorder;

use boomerang::runtime;

#[derive(serde::Serialize, serde::Deserialize)]
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

#[derive(serde::Deserialize)]
struct ReplayRecord<'a>(&'a str, runtime::ActionKey, runtime::Tag);
