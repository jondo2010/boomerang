//! Capture asynchronous key presses, and sends them through an output port.
//!
use boomerang::{
    builder::{BuilderReactionKey, Physical, TypedActionKey, TypedPortKey},
    runtime, Reactor,
};

use std::{io::Stdout, ops::DerefMut};
pub use termion::event::Key;
use termion::raw::{IntoRawMode, RawTerminal};

#[derive(Reactor)]
#[reactor(state = "KeyboardEvents")]
pub struct KeyboardEventsBuilder {
    /// The latest key press.
    #[reactor(output())]
    pub arrow_key_pressed: TypedPortKey<Key>,

    #[reactor(action(physical, min_delay = "100 msec"))]
    key_press: TypedActionKey<Key, Physical>,

    #[reactor(reaction(function = "KeyboardEvents::reaction_key_press"))]
    key_press_reaction: BuilderReactionKey,

    #[reactor(reaction(function = "KeyboardEvents::reaction_shutdown"))]
    shutdown_reaction: BuilderReactionKey,

    #[reactor(reaction(function = "KeyboardEvents::reaction_startup"))]
    startup_reaction: BuilderReactionKey,
}

pub struct KeyboardEvents {
    raw_terminal: Option<RawTerminal<Stdout>>,
}

impl Default for KeyboardEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardEvents {
    pub fn new() -> Self {
        Self { raw_terminal: None }
    }

    #[boomerang::reaction(reactor = "KeyboardEventsBuilder")]
    fn reaction_key_press(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(triggers)] key_press: runtime::PhysicalActionRef<Key>,
        #[reactor::port(effects)] arrow_key_pressed: &mut runtime::Port<Key>,
    ) {
        *arrow_key_pressed.deref_mut() = ctx.get_action(&key_press);
    }

    #[boomerang::reaction(reactor = "KeyboardEventsBuilder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
        drop(self.raw_terminal.take()); // exit raw mode
    }

    #[boomerang::reaction(reactor = "KeyboardEventsBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut key_press: runtime::PhysicalActionRef<Key>,
    ) {
        let stdin = std::io::stdin();

        // enter raw mode, to get key presses one by one
        // this will stay so until this variable is dropped
        self.raw_terminal = Some(std::io::stdout().into_raw_mode().unwrap());

        let mut send_ctx = ctx.make_send_context();
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
