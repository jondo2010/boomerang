//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.

use std::io::Write;

use boomerang::{builder::BuilderReactionKey, runtime, Reactor};
use boomerang_util::keyboard_events::{KeyboardEvents, KeyboardEventsBuilder};
use termion::event::Key;

/// A simple Reactor that triggers on key_press events.
/// It reads keyboard input and prints the key that was pressed.
#[derive(Clone, Reactor)]
#[reactor(state = "State")]
struct Builder {
    /// this thing helps capturing key presses
    #[reactor(child(state = "KeyboardEvents::new()"))]
    keyboard: KeyboardEventsBuilder,

    #[reactor(reaction(function = "State::reaction_key_press"))]
    key_press_reaction: BuilderReactionKey,
}

struct State;

impl State {
    #[boomerang::reaction(reactor = "Builder")]
    fn reaction_key_press(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(triggers, path = "keyboard.arrow_key_pressed")]
        arrow_key_pressed: &runtime::Port<Key>,
    ) {
        // this might be overwritten several times, only committed on screen refreshes
        match arrow_key_pressed.get().unwrap() {
            Key::Left => print!("←"),
            Key::Right => print!("→"),
            Key::Up => print!("↑"),
            Key::Down => print!("↓"),
            _ => unreachable!(),
        };
        std::io::stdout().flush().unwrap();
    }
}

#[cfg(not(windows))]
fn main() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_run_reactor::<Builder>("printer", State).unwrap();
}

#[cfg(windows)]
fn main() {}
