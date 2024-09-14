//! Capture asynchronous key presses, and sends them through an output port.
use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

use std::{io::Stdout, ops::DerefMut};
pub use termion::event::Key;
use termion::raw::{IntoRawMode, RawTerminal};

#[derive(Reactor, Clone)]
#[reactor(state = KeyboardEvents)]
pub struct KeyboardEventsBuilder {
    /// The latest key press.
    pub arrow_key_pressed: TypedPortKey<Key, Output>,

    #[reactor(action(min_delay = "10 msec"))]
    key_press: TypedActionKey<Key, Physical>,

    key_press_reaction: TypedReactionKey<ReactionKeyPress<'static>>,
    shutdown_reaction: TypedReactionKey<ReactionShutdown>,
    startup_reaction: TypedReactionKey<ReactionStartup>,
}

#[derive(Default)]
pub struct KeyboardEvents {
    raw_terminal: Option<RawTerminal<Stdout>>,
}

#[derive(Reaction)]
struct ReactionKeyPress<'a> {
    #[reaction(triggers)]
    key_press: runtime::PhysicalActionRef<Key>,
    arrow_key_pressed: runtime::OutputRef<'a, Key>,
}

impl<'a> Trigger for ReactionKeyPress<'a> {
    type Reactor = KeyboardEventsBuilder;

    fn trigger(&mut self, ctx: &mut runtime::Context, _state: &mut KeyboardEvents) {
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

fn __trigger_inner(
    ctx: &mut ::boomerang::runtime::Context,
    state: &mut dyn ::boomerang::runtime::ReactorState,
    ports: &[::boomerang::runtime::PortRef],
    ports_mut: &mut [::boomerang::runtime::PortRefMut],
    actions: &mut [&mut ::boomerang::runtime::Action],
) {
    let state: &mut <<ReactionKeyPress as Trigger> ::Reactor as ::boomerang::builder::Reactor> ::State = state.downcast_mut().expect("Unable to downcast reactor state");
    let [key_press]: &mut [&mut ::boomerang::runtime::Action; 1usize] =
        ::std::convert::TryInto::try_into(actions)
            .expect("Unable to destructure actions for reaction");
    let key_press = (*key_press).into();
    let [arrow_key_pressed]: &mut [::boomerang::runtime::PortRefMut; 1usize] =
        ::std::convert::TryInto::try_into(ports_mut)
            .expect("Unable to destructure mut ports for reaction");
    let arrow_key_pressed = arrow_key_pressed
        .downcast_mut::<runtime::Port<_>>()
        .map(Into::into)
        .expect("Wrong Port type for reaction");
    ReactionKeyPress {
        key_press,
        arrow_key_pressed,
    }
    .trigger(ctx, state);
}
