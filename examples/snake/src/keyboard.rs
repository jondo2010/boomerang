//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.
//!
//! Pressing arrow keys will print them to the terminal.

#[cfg(not(windows))]
mod keyboard_events;

#[cfg(not(windows))]
mod example {
    use std::io::Write;

    use crate::keyboard_events::{KeyboardEvents, KeyboardEventsBuilder};
    use boomerang::prelude::*;

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

    impl runtime::Trigger<()> for ReactionKeyPress<'_> {
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
    use boomerang::prelude::*;
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();
    let _reactor = example::Example::build("printer", (), None, None, &mut env_builder).unwrap();

    let (env, triggers, _) = env_builder.into_runtime_parts().unwrap();

    let config = runtime::Config::default()
        .with_fast_forward(false)
        .with_keep_alive(true);
    let mut sched = runtime::Scheduler::new(env, triggers, config);
    sched.event_loop();
}

#[cfg(windows)]
fn main() {}
