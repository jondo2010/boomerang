//! Each timer request delivers two values through a physical action.
use boomerang::prelude::*;
use boomerang::runtime::context::TrySendError;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};

/// A worker that owns callback requests and joins its thread when dropped.
#[derive(Debug)]
struct CallbackWorker {
    requests: Option<mpsc::Sender<usize>>,
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl CallbackWorker {
    fn new(init: CallbackInit) -> Self {
        let (requests, incoming) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let thread = std::thread::spawn(move || {
            while let Ok(request) = incoming.recv() {
                if stop.load(Ordering::Acquire) {
                    return;
                }
                // Simulate asynchronous work requested by a timer reaction.
                // The work can finish after the scheduler timeout.
                std::thread::sleep(std::time::Duration::from_millis(100));
                for callback in 0..2 {
                    let Ok(mut prepared) =
                        init.send
                            .prepare_action(&init.a, request * 2 + callback, None)
                    else {
                        return;
                    };
                    loop {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        match prepared.try_send() {
                            Ok(()) => break,
                            Err(TrySendError::Closed(_)) => return,
                            Err(TrySendError::Full(retry)) => prepared = retry,
                        }
                        std::thread::park_timeout(std::time::Duration::from_millis(1));
                    }
                }
            }
        });
        Self {
            requests: Some(requests),
            cancelled,
            thread: Some(thread),
        }
    }
    fn request(&self, request: usize) {
        self.requests.as_ref().unwrap().send(request).unwrap();
    }
}
impl Drop for CallbackWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.requests.take();
        if let Some(worker) = self.thread.take() {
            worker.join().unwrap();
        }
    }
}
#[derive(Debug)]
struct State {
    worker: CallbackWorker,
    expected_time: Duration,
    toggle: bool,
    received: usize,
    requested: usize,
}
impl State {
    fn new(init: CallbackInit) -> Result<Self, std::convert::Infallible> {
        Ok(Self {
            worker: CallbackWorker::new(init),
            expected_time: Duration::milliseconds(100),
            toggle: false,
            received: 0,
            requested: 0,
        })
    }
}
#[reactor(state = State, state_init = State::new, init_context(name = CallbackInit, actions = [a]))]
fn AsyncCallback(#[physical_action] a: usize) -> impl Reactor {
    let t = ctx.add_timer(
        "t",
        TimerSpec::default().with_period(Duration::milliseconds(200)),
    )?;
    ctx.add_reaction(Some("Request"))
        .with_trigger(t)
        .with_reaction_fn(|_ctx, state, _| {
            if state.requested < 10 {
                state.worker.request(state.requested);
                state.requested += 1;
            }
        })
        .finish()?;
    reaction! { Callback(a) {
        let elapsed = ctx.get_elapsed_logical_time();
        assert!(elapsed > state.expected_time, "callback must follow its requested work");
        assert_eq!(ctx.get_action_value(&mut a), Some(&state.received));
        state.received += 1;
        if state.received == 20 { ctx.schedule_shutdown(None); }
        if state.toggle { state.expected_time += Duration::milliseconds(200); }
        state.toggle = !state.toggle;
    } }
}
#[test]
fn async_callback() {
    let config = runtime::Config::default().with_timeout(Duration::seconds(30));
    let (_, envs) = boomerang_util::runner::build_and_test_reactor_initialized(
        AsyncCallback(),
        "async_callback",
        config,
    )
    .unwrap();
    let state = envs[0]
        .reactors
        .values()
        .find_map(|r| r.get_state::<State>())
        .unwrap();
    assert_eq!(
        state.received, 20,
        "ten timer requests each deliver two separate callbacks"
    );
    // Dropping state cancels the worker and joins its thread.
    drop(envs);
}

#[test]
fn callback_finishing_after_timeout_observes_closed_mailbox() {
    let config = runtime::Config::default().with_timeout(Duration::milliseconds(20));
    let (_, envs) = boomerang_util::runner::build_and_test_reactor_initialized(
        AsyncCallback(),
        "late_callback",
        config,
    )
    .unwrap();
    let state = envs[0]
        .reactors
        .values()
        .find_map(|r| r.get_state::<State>())
        .unwrap();
    assert_eq!(state.received, 0);
    // Keep state alive until the requested callback finishes.
    // The worker returns when the runtime closes the mailbox.
    let mut envs = envs;
    let state = envs[0]
        .reactors
        .values_mut()
        .find_map(|r| r.get_state_mut::<State>())
        .unwrap();
    state.worker.requests.take();
    state.worker.thread.take().unwrap().join().unwrap();
}
