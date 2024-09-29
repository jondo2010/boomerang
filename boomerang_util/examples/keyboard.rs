//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.
//!
//! Pressing arrow keys will print them to the terminal.

#[cfg(not(windows))]
mod example {
    use std::io::Write;

    use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
    use boomerang_util::keyboard_events::{KeyboardEvents, KeyboardEventsBuilder};

    /// A simple Reactor that triggers on key_press events.
    /// It reads keyboard input and prints the key that was pressed.
    #[derive(Reactor)]
    #[reactor(state = "()", reaction = "ReactionKeyPress")]
    pub struct Example {
        /// this thing helps capturing key presses
        #[reactor(child = KeyboardEvents::default())]
        keyboard: KeyboardEventsBuilder,
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Example")]
    struct ReactionKeyPress<'a> {
        #[reaction(path = "keyboard.arrow_key_pressed")]
        arrow_key_pressed: runtime::InputRef<'a, termion::event::Key>,
    }

    impl Trigger<Example> for ReactionKeyPress<'_> {
        fn trigger(self, _ctx: &mut runtime::Context, _: &mut ()) {
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();

            // this might be overwritten several times, only committed on screen refreshes
            let c = match *self.arrow_key_pressed {
                Some(termion::event::Key::Left) => '←',
                Some(termion::event::Key::Right) => '→',
                Some(termion::event::Key::Up) => '↑',
                Some(termion::event::Key::Down) => '↓',
                _ => unreachable!(),
            };

            // Move cursor back one position and clear the last character
            write!(stdout, "\x1B[1D\x1B[K{c}").unwrap();
            stdout.flush().unwrap();
        }
    }
}

#[cfg(not(windows))]
fn main() {
    tracing_subscriber::fmt::init();
    let _ =
        boomerang_util::runner::build_and_run_reactor::<example::Example>("printer", ()).unwrap();
}

#[cfg(windows)]
fn main() {}
