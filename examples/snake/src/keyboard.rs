//! This example shows how to use the `KeyboardEvents` reactor to read keyboard input.
//!
//! Pressing arrow keys will print them to the terminal.

mod keyboard_events;

/// A simple Reactor that triggers on key_press events.
/// It reads keyboard input and prints the key that was pressed.
mod example {
    use std::io::Write;

    use crate::keyboard_events::{KeyboardEvents, KeyboardEventsState};
    use boomerang::prelude::*;
    use crossterm::{
        cursor::MoveLeft,
        event::KeyCode,
        execute,
        terminal::{Clear, ClearType},
    };

    #[reactor]
    pub fn Example() -> impl Reactor<()> {
        let keyboard = builder.add_child_reactor(
            KeyboardEvents(),
            "keyboard",
            KeyboardEventsState::default(),
            false,
        )?;

        builder
            .add_reaction(Some("ReactionKeyPress"))
            .with_trigger(keyboard.arrow_key_pressed)
            .with_reaction_fn(|_ctx, _state, (key_event,)| {
                let mut stdout = std::io::stdout();

                // this might be overwritten several times, only committed on screen refreshes
                let c = match key_event.as_ref().map(|k| k.code) {
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
            })
            .finish()?;
    }
}

fn main() {
    use boomerang::prelude::*;
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();
    let _ = example::Example()
        .build("printer", (), None, None, false, &mut env_builder)
        .unwrap();

    let config = runtime::Config::default()
        .with_fast_forward(false)
        .with_keep_alive(true);
    let BuilderRuntimeParts {
        enclaves,
        aliases: _,
        ..
    } = env_builder.into_runtime_parts(&config).unwrap();

    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    let mut sched = runtime::Scheduler::new(enclave_key, enclave, config);
    sched.event_loop();
}
