//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.
//!
//! Pressing arrow keys will print them to the terminal.

use std::io::Write;

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
use boomerang_util::keyboard_events::{KeyboardEvents, KeyboardEventsBuilder};
use termion::event::Key;

/// A simple Reactor that triggers on key_press events.
/// It reads keyboard input and prints the key that was pressed.
#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct Builder {
    /// this thing helps capturing key presses
    #[reactor(child = KeyboardEvents::default())]
    keyboard: KeyboardEventsBuilder,

    key_press_reaction: TypedReactionKey<ReactionKeyPress<'static>>,
}

#[derive(Reaction)]
struct ReactionKeyPress<'a> {
    #[reaction(path = "keyboard.arrow_key_pressed")]
    arrow_key_pressed: &'a runtime::Port<Key>,
}

impl Trigger for ReactionKeyPress<'_> {
    type Reactor = Builder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _: &mut ()) {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        // this might be overwritten several times, only committed on screen refreshes
        let c = match self.arrow_key_pressed.get().unwrap() {
            Key::Left => '←',
            Key::Right => '→',
            Key::Up => '↑',
            Key::Down => '↓',
            _ => unreachable!(),
        };

        // Move cursor back one position and clear the last character
        write!(stdout, "\x1B[1D\x1B[K{c}").unwrap();
        stdout.flush().unwrap();
    }
}

#[cfg(not(windows))]
fn main() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_run_reactor::<Builder>("printer", ()).unwrap();
}

#[cfg(windows)]
fn main() {}
