//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.
#[cfg(not(windows))]
use boomerang_util::keyboard_events::{KeyboardEvents, KeyboardEventsBuilder};

#[cfg(not(windows))]
fn main() {
    tracing_subscriber::fmt::init();
    let _ = boomerang::run::build_and_run_reactor::<KeyboardEventsBuilder>(
        "keyboard_events",
        KeyboardEvents::new(),
    )
    .unwrap();
}

#[cfg(windows)]
fn main() {}
