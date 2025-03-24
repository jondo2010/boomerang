//! Capture asynchronous key presses, and sends them through an output port.
use boomerang::prelude::*;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};

#[derive(Default)]
pub struct KeyboardEvents {
    raw_mode_enabled: bool,
}

impl std::fmt::Debug for KeyboardEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyboardEvents").finish()
    }
}

#[derive(Reactor, Clone)]
#[reactor(
    state = "KeyboardEvents",
    reaction = "ReactionKeyPress",
    reaction = "ReactionShutdown",
    reaction = "ReactionStartup"
)]
pub struct KeyboardEventsBuilder {
    /// The latest key press.
    pub arrow_key_pressed: TypedPortKey<KeyEvent, Output>,

    #[reactor(action(min_delay = "10 msec", replay = "record-replay"))]
    key_press: TypedActionKey<KeyEvent, Physical>,
}

#[derive(Reaction)]
#[reaction(reactor = "KeyboardEventsBuilder")]
struct ReactionKeyPress<'a> {
    #[reaction(triggers)]
    key_press: runtime::ActionRef<'a, KeyEvent>,
    arrow_key_pressed: runtime::OutputRef<'a, KeyEvent>,
}

impl<'a> runtime::Trigger<KeyboardEvents> for ReactionKeyPress<'a> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut KeyboardEvents) {
        *self.arrow_key_pressed = ctx.get_action_value(&mut self.key_press).cloned();
    }
}

#[derive(Reaction)]
#[reaction(reactor = "KeyboardEventsBuilder", triggers(shutdown))]
struct ReactionShutdown;

impl runtime::Trigger<KeyboardEvents> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut KeyboardEvents) {
        if state.raw_mode_enabled {
            let _ = disable_raw_mode(); // exit raw mode
            state.raw_mode_enabled = false;
        }
    }
}

#[derive(Reaction)]
#[reaction(reactor = "KeyboardEventsBuilder", triggers(startup))]
struct ReactionStartup {
    key_press: runtime::AsyncActionRef<KeyEvent>,
}

impl runtime::Trigger<KeyboardEvents> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut KeyboardEvents) {
        // enter raw mode, to get key presses one by one
        enable_raw_mode().unwrap();
        state.raw_mode_enabled = true;

        let mut send_ctx = ctx.make_send_context();

        std::thread::spawn(move || loop {
            if let Ok(Event::Key(key_event)) = event::read() {
                match (key_event.code, key_event.modifiers) {
                    (KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down, _) => {
                        tracing::debug!("received {:?}", key_event);
                        send_ctx.schedule_action_async(&self.key_press, key_event, None);
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        tracing::debug!("Ctrl-C received, shutting down.");
                        send_ctx.schedule_shutdown(None);
                        break;
                    }
                    _ => {
                        tracing::trace!("received {:?}", key_event);
                    }
                }
            }
        });
    }
}
