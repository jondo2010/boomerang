//! Capture asynchronous key presses, and sends them through an output port.
use boomerang::prelude::*;

use std::io::Stdout;
pub use termion::event::Key;
use termion::raw::{IntoRawMode, RawTerminal};

#[derive(Default)]
pub struct KeyboardEvents {
    raw_terminal: Option<RawTerminal<Stdout>>,
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
    pub arrow_key_pressed: TypedPortKey<Key, Output>,

    #[reactor(action(min_delay = "10 msec"))]
    key_press: TypedActionKey<Key, Physical>,
}

#[derive(Reaction)]
#[reaction(reactor = "KeyboardEventsBuilder")]
struct ReactionKeyPress<'a> {
    #[reaction(triggers)]
    key_press: runtime::ActionRef<'a, Key>,
    arrow_key_pressed: runtime::OutputRef<'a, Key>,
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
        drop(state.raw_terminal.take()); // exit raw mode
    }
}

#[derive(Reaction)]
#[reaction(reactor = "KeyboardEventsBuilder", triggers(startup))]
struct ReactionStartup {
    key_press: runtime::AsyncActionRef<Key>,
}

impl runtime::Trigger<KeyboardEvents> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut KeyboardEvents) {
        let stdin = std::io::stdin();

        // enter raw mode, to get key presses one by one
        // this will stay so until this variable is dropped
        state.raw_terminal = Some(std::io::stdout().into_raw_mode().unwrap());

        let mut send_ctx = ctx.make_send_context();

        std::thread::spawn(move || {
            use termion::input::TermRead;

            for c in stdin.keys() {
                match c.unwrap() {
                    k @ (Key::Left | Key::Right | Key::Up | Key::Down) => {
                        tracing::debug!("received {:?}", k);
                        send_ctx.schedule_action_async(&self.key_press, k, None);
                    }
                    Key::Ctrl('c') => {
                        tracing::debug!("Ctrl-C received, shutting down.");
                        send_ctx.schedule_shutdown(None);
                        break;
                    }
                    k => {
                        tracing::trace!("received {:?}", k);
                    }
                }
            }
        });
    }
}
