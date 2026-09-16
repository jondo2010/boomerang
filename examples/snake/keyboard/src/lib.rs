//! Owning Cargo crate for the terminal keyboard and arrow display components.

boomerang::component! {
pub mod keyboard {

mod input;

use input::KeyboardInput;
use boomerang::prelude::*;
use crossterm::event::KeyEvent;

#[derive(Default)]
pub struct KeyboardState {
    terminal: KeyboardInput,
}

#[reactor(
    state = KeyboardState,
    state_init = KeyboardState::default,
    contract = "snake.keyboard",
    contract_version = 1,
    bounds(
        queue_capacity = 1024,
        payload_bytes = 32768,
        state_bytes = 1024,
        scratch_bytes = 1024
    )
)]
pub fn Keyboard(
    #[physical_action(min_delay = 10 msec)] key_press: KeyEvent,
    #[output] key: KeyEvent,
    #[output] ready: (),
) -> impl Reactor {
    reaction! {
        Startup (startup) -> key_press, ready {
            state.terminal.start(ctx.make_send_context(), key_press.to_async());
            *ready = Some(());
        }
    }
    reaction! {
        KeyPress (key_press) -> key {
            *key = ctx.get_action_value(&mut key_press).copied();
        }
    }
    reaction! {
        Shutdown (shutdown) {
            state.terminal.stop();
        }
    }
}
}
}

boomerang::component! {
pub mod display {
use boomerang::prelude::*;
use crossterm::{
    cursor::MoveLeft,
    event::{KeyCode, KeyEvent},
    execute,
    terminal::{Clear, ClearType},
};
use std::io::Write;

#[reactor(
    contract = "snake.keyboard-display",
    contract_version = 1,
    bounds(
        queue_capacity = 16,
        payload_bytes = 1024,
        state_bytes = 0,
        scratch_bytes = 1024
    )
)]
pub fn ArrowDisplay(#[input] key: KeyEvent) -> impl Reactor {
    reaction! {
        Display (key) {
            let arrow = match key.as_ref().map(|event| event.code) {
                Some(KeyCode::Left) => '←',
                Some(KeyCode::Right) => '→',
                Some(KeyCode::Up) => '↑',
                Some(KeyCode::Down) => '↓',
                _ => return,
            };
            let mut stdout = std::io::stdout();
            execute!(stdout, MoveLeft(1), Clear(ClearType::UntilNewLine)).unwrap();
            write!(stdout, "{arrow}").unwrap();
            stdout.flush().unwrap();
        }
    }
}
}
}
