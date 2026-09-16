//! Own the terminal mode and the thread that feeds the keyboard physical action.

use boomerang::runtime::{
    ActionCommon, AsyncActionRef, AsyncEvent, AsyncEventTarget, CommonContext, SendContext,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Default)]
pub struct KeyboardInput {
    raw_terminal: bool,
    stop_requested: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl KeyboardInput {
    pub fn start(&mut self, send_ctx: SendContext, action: AsyncActionRef<KeyEvent>) {
        terminal::enable_raw_mode().expect("keyboard input requires a terminal");
        self.raw_terminal = true;
        self.stop_requested.store(false, Ordering::Relaxed);
        let stop_requested = self.stop_requested.clone();
        self.thread = Some(std::thread::spawn(move || {
            let result = (|| -> std::io::Result<()> {
                while !stop_requested.load(Ordering::Relaxed) {
                    if !event::poll(Duration::from_millis(50))? {
                        continue;
                    }
                    let Event::Key(key) = event::read()? else {
                        continue;
                    };
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    match (key.code, key.modifiers) {
                        (KeyCode::Char('c'), modifiers)
                            if modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            send_until_stopped(&send_ctx, &stop_requested, || {
                                AsyncEvent::Shutdown {
                                    delay: Default::default(),
                                }
                            });
                            break;
                        }
                        (KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down, _) => {
                            let time = send_ctx.get_physical_time() + action.min_delay();
                            if !send_until_stopped(&send_ctx, &stop_requested, || {
                                AsyncEvent::Physical {
                                    time,
                                    target: AsyncEventTarget::Action(action.key()),
                                    value: Box::new(key),
                                }
                            }) {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                tracing::error!(%error, "keyboard input failed");
                send_until_stopped(&send_ctx, &stop_requested, || AsyncEvent::Shutdown {
                    delay: Default::default(),
                });
            }
        }));
    }

    pub fn stop(&mut self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if self.raw_terminal {
            let _ = terminal::disable_raw_mode();
            self.raw_terminal = false;
        }
    }
}

fn send_until_stopped(
    sender: &SendContext,
    stop_requested: &AtomicBool,
    event: impl Fn() -> AsyncEvent,
) -> bool {
    // A blocking send can deadlock a shutdown reaction that joins this thread.
    while !stop_requested.load(Ordering::Relaxed) {
        if let Some(accepted) = sender.try_schedule_async(event()) {
            return accepted;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

impl Drop for KeyboardInput {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boomerang::runtime::{AsyncEvent, Enclave, EnclaveKey};

    #[test]
    fn stopping_input_cancels_a_send_to_a_full_scheduler_mailbox() {
        let enclave = Enclave::with_event_q_size(1);
        let sender = enclave.create_send_context(EnclaveKey::default());
        let shutdown = || AsyncEvent::Shutdown {
            delay: Default::default(),
        };
        assert!(sender.schedule_external(shutdown()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let accepted = send_until_stopped(&sender, &worker_stop, shutdown);
            done_tx.send(accepted).unwrap();
        });
        std::thread::sleep(Duration::from_millis(20));
        stop.store(true, Ordering::Relaxed);
        let result = done_rx.recv_timeout(Duration::from_secs(1));
        // Release even a broken blocking sender before failing the assertion.
        drop(enclave);
        worker.join().unwrap();
        assert_eq!(result, Ok(false));
    }
}
