//! Capture asynchronous key presses, and sends them through an output port.
use boomerang::{
    builder::{Physical, Trigger, TypedActionKey, TypedPortKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

use std::{io::Stdout, ops::DerefMut};
pub use termion::event::Key;
use termion::raw::{IntoRawMode, RawTerminal};

#[derive(Reactor, Clone)]
#[reactor(state = KeyboardEvents)]
pub struct KeyboardEventsBuilder {
    /// The latest key press.
    #[reactor(port = "output")]
    pub arrow_key_pressed: TypedPortKey<Key>,

    #[reactor(action(physical, min_delay = "10 msec"))]
    key_press: TypedActionKey<Key, Physical>,

    key_press_reaction: TypedReactionKey<ReactionKeyPress<'static>>,

    shutdown_reaction: TypedReactionKey<ReactionShutdown>,

    startup_reaction: TypedReactionKey<ReactionStartup>,
}

pub struct KeyboardEvents {
    raw_terminal: Option<RawTerminal<Stdout>>,
}

impl Default for KeyboardEvents {
    fn default() -> Self {
        Self { raw_terminal: None }
    }
}

#[derive(Reaction)]
struct ReactionKeyPress<'a> {
    #[reaction(triggers)]
    key_press: runtime::PhysicalActionRef<Key>,
    arrow_key_pressed: &'a mut runtime::Port<Key>,
}

impl<'a> Trigger for ReactionKeyPress<'a> {
    type Reactor = KeyboardEventsBuilder;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut KeyboardEvents) {
        *self.arrow_key_pressed.deref_mut() = ctx.get_action(&mut self.key_press);
    }
}

#[derive(Reaction)]
#[reaction(triggers(shutdown))]
struct ReactionShutdown;

impl Trigger for ReactionShutdown {
    type Reactor = KeyboardEventsBuilder;

    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut KeyboardEvents) {
        drop(state.raw_terminal.take()); // exit raw mode
    }
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct ReactionStartup {
    key_press: runtime::PhysicalActionRef<Key>,
}

impl Trigger for ReactionStartup {
    type Reactor = KeyboardEventsBuilder;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut KeyboardEvents) {
        let stdin = std::io::stdin();

        // enter raw mode, to get key presses one by one
        // this will stay so until this variable is dropped
        state.raw_terminal = Some(std::io::stdout().into_raw_mode().unwrap());

        let mut send_ctx = ctx.make_send_context();
        let mut key_press = self.key_press.clone();

        std::thread::spawn(move || {
            use termion::input::TermRead;

            for c in stdin.keys() {
                match c.unwrap() {
                    k @ (Key::Left | Key::Right | Key::Up | Key::Down) => {
                        tracing::debug!("received {:?}", k);
                        send_ctx.schedule_action(&mut key_press, Some(k), None);
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
