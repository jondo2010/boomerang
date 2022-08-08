//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.
//!
use boomerang_util::keyboard_events::{KeyboardEvents, KeyboardEventsBuilder};

fn main() {
    tracing_subscriber::fmt::init();
    let _ = boomerang::run::build_and_run_reactor::<KeyboardEventsBuilder>(
        "keyboard_events",
        KeyboardEvents::new(),
    )
    .unwrap();
}
