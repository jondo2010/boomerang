//! Terminal mode and worker thread for the keyboard physical action.

use boomerang::runtime::{
    context::{PreparedAction, SendContext, TrySendError, TryShutdownError},
    AsyncActionRef,
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
    pub fn start(send_ctx: SendContext, action: AsyncActionRef<KeyEvent>) -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut input = Self {
            raw_terminal: true,
            stop_requested: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let stop_requested = input.stop_requested.clone();
        input.thread = Some(
            std::thread::Builder::new()
                .name("keyboard-input".into())
                .spawn(move || {
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
                                    request_shutdown_until_stopped(&send_ctx, &stop_requested);
                                    break;
                                }
                                (
                                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down,
                                    _,
                                ) => {
                                    let prepared = send_ctx
                                        .prepare_action(&action, key, None)
                                        .map_err(std::io::Error::other)?;
                                    if !send_until_stopped(prepared, &stop_requested) {
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
                        request_shutdown_until_stopped(&send_ctx, &stop_requested);
                    }
                })?,
        );
        Ok(input)
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

/// Retry delivery of a key without changing its prepared timestamp or payload.
fn send_until_stopped(mut prepared: PreparedAction<KeyEvent>, stop_requested: &AtomicBool) -> bool {
    while !stop_requested.load(Ordering::Relaxed) {
        match prepared.try_send() {
            Ok(()) => return true,
            Err(TrySendError::Closed(_)) => return false,
            Err(TrySendError::Full(retry)) => prepared = retry,
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
    false
}

fn request_shutdown_until_stopped(sender: &SendContext, stop_requested: &AtomicBool) -> bool {
    // Do not block a worker that the shutdown reaction must join.
    while !stop_requested.load(Ordering::Relaxed) {
        match sender.try_shutdown(None) {
            Ok(()) => return true,
            Err(TryShutdownError::Closed) => return false,
            Err(TryShutdownError::Full) => {}
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
    use boomerang::runtime::{Enclave, EnclaveKey};

    #[test]
    fn stopping_input_cancels_shutdown_to_a_full_scheduler_mailbox() {
        let enclave = Enclave::with_event_q_size(1);
        let sender = enclave.create_send_context(EnclaveKey::default());
        sender.try_shutdown(None).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let accepted = request_shutdown_until_stopped(&sender, &worker_stop);
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

    #[test]
    fn stopping_input_cancels_a_typed_send_to_a_full_scheduler_mailbox() {
        use boomerang::runtime::{Action, Config, Scheduler};
        let mut enclave = Enclave::with_event_q_size(1);
        let key =
            enclave.insert_action(|key| Action::<KeyEvent>::new("key", key, None, false).boxed());
        let action = enclave.create_async_action_ref::<KeyEvent>(key);
        let sender = enclave.create_send_context(EnclaveKey::default());
        let mut scheduler = Scheduler::new(EnclaveKey::default(), enclave, Config::default(), None);
        scheduler.startup();
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        sender
            .prepare_action(&action, key, None)
            .unwrap()
            .try_send()
            .unwrap();
        let prepared = sender.prepare_action(&action, key, None).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (full_tx, full_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let Err(TrySendError::Full(prepared)) = prepared.try_send() else {
                panic!("mailbox must be full");
            };
            full_tx.send(()).unwrap();
            done_tx
                .send(send_until_stopped(prepared, &worker_stop))
                .unwrap();
        });
        full_rx.recv().unwrap();
        stop.store(true, Ordering::Relaxed);
        let result = done_rx.recv_timeout(Duration::from_secs(1));
        // Close the mailbox before joining, in case the sender blocks.
        drop(scheduler);
        worker.join().unwrap();
        assert_eq!(result, Ok(false));
    }
}
