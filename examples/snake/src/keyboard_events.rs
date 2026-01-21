//! Capture asynchronous key presses, and sends them through an output port.
use boomerang::prelude::*;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal,
};

#[reactor]
pub fn KeyboardEvents(
    #[output] arrow_key_pressed: KeyEvent,
    #[state] raw_terminal: bool,
) -> impl Reactor<KeyboardEventsState, Ports = KeyboardEventsPorts> {
    let key_press =
        builder.add_physical_action::<KeyEvent>("key_press", Some(Duration::milliseconds(10)))?;

    builder.add_action_recorder(key_press)?;
    builder.add_action_replayer(key_press)?;

    builder
        .add_reaction(Some("ReactionKeyPress"))
        .with_trigger(key_press)
        .with_effect(arrow_key_pressed)
        .with_reaction_fn(|_ctx, _state, (mut key_event, mut arrow_key_pressed)| {
            *arrow_key_pressed = _ctx.get_action_value(&mut key_event).cloned();
        })
        .finish()?;

    builder
        .add_reaction(Some("ReactionShutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, _| {
            if state.raw_terminal {
                let _ = terminal::disable_raw_mode(); // exit raw mode
                state.raw_terminal = false;
            }
        })
        .finish()?;

    builder
        .add_reaction(Some("ReactionStartup"))
        .with_startup_trigger()
        .with_effect(key_press)
        .with_reaction_fn(|_ctx, state, (_, key_press)| {
            // enter raw mode, to get key presses one by one
            terminal::enable_raw_mode().unwrap();
            state.raw_terminal = true;

            let mut send_ctx = _ctx.make_send_context();
            let async_key_press = key_press.to_async();

            std::thread::spawn(move || loop {
                if let Ok(Event::Key(key_event)) = event::read() {
                    match (key_event.code, key_event.modifiers) {
                        (KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down, _) => {
                            tracing::debug!("received {:?}", key_event);
                            send_ctx.schedule_action_async(&async_key_press, key_event, None);
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
        })
        .finish()?;
}
