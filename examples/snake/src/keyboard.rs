//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.
//!
//! Pressing arrow keys will print them to the terminal.

#[cfg(any(unix, windows))]
mod keyboard_events;

#[cfg(any(unix, windows))]
mod example {
    use std::io::Write;

    use crate::keyboard_events::{KeyboardEvents, KeyboardEventsBuilder};
    use boomerang::prelude::*;
    use crossterm::{
        cursor::MoveLeft,
        event::{KeyCode, KeyEvent},
        execute,
        terminal::{Clear, ClearType},
    };

    /// A simple Reactor that triggers on key_press events.
    /// It reads keyboard input and prints the key that was pressed.
    #[derive(Reactor)]
    #[reactor(state = (), reaction = "ReactionKeyPress")]
    pub struct Example {
        /// this thing helps capturing key presses
        #[reactor(child(state = KeyboardEvents::default()))]
        keyboard: KeyboardEventsBuilder,
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Example")]
    struct ReactionKeyPress<'a> {
        #[reaction(path = "keyboard.arrow_key_pressed")]
        arrow_key_pressed: runtime::InputRef<'a, KeyEvent>,
    }

    impl runtime::Trigger<()> for ReactionKeyPress<'_> {
        fn trigger(self, _ctx: &mut runtime::Context, _: &mut ()) {
            let mut stdout = std::io::stdout();

            // this might be overwritten several times, only committed on screen refreshes
            let c = match self.arrow_key_pressed.as_ref().map(|k| k.code) {
                Some(KeyCode::Left) => '←',
                Some(KeyCode::Right) => '→',
                Some(KeyCode::Up) => '↑',
                Some(KeyCode::Down) => '↓',
                _ => unreachable!(),
            };

            // Move cursor back one position and clear the last character
            execute!(stdout, MoveLeft(1), Clear(ClearType::UntilNewLine)).unwrap();
            write!(stdout, "{c}").unwrap();
            stdout.flush().unwrap();
        }
    }
}

fn main() {
    use boomerang::prelude::*;
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();
    let _reactor =
        example::Example::build("printer", (), None, None, false, &mut env_builder).unwrap();

    let BuilderRuntimeParts {
        enclaves,
        aliases: _,
        ..
    } = env_builder.into_runtime_parts().unwrap();

    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    let config = runtime::Config::default()
        .with_fast_forward(false)
        .with_keep_alive(true);
    let mut sched = runtime::Scheduler::new(enclave_key, enclave, config);
    sched.event_loop();
}
