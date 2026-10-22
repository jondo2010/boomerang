//! One sensor declaration, expanded as host metadata or target payload.
//!
//! The reactor signature and attributes define metadata shared by both facets.
//! Payload emission also includes reaction bodies and the driver implementation.
//! Metadata emission omits those runtime-only items.

use crate::component;

component! {
    /// Owns the sample driver and its typed physical action.
    pub mod sensor {
        use boomerang::prelude::*;

        // Both facets use the reactor declaration; only payload includes reaction bodies.
        /// A sample acquired at 10 ms is delivered at 11 ms by the action delay.
        #[reactor(
            state = DriverState, state_init = DriverState::new,
            init_context(name = DriverInit, source = "plant", actions = [sample], clock_control = true),
            contract = "example.physical-input", contract_version = 1,
            bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 256, scratch_bytes = 64)
        )]
        pub fn Sensor(#[physical_action(min_delay = 1 msec)] sample: u32) -> impl Reactor {
            reaction! {
                receive (sample) {
                    let value = *ctx.get_action_value(&mut sample).expect("sample triggered reaction");
                    let logical_ms = ctx.get_elapsed_logical_time().whole_milliseconds();
                    let physical_ms = ctx.try_get_physical_time().expect("clock is running").0 / 1_000_000;
                    println!("received {value} at logical {logical_ms} ms (physical {physical_ms} ms)");
                    ctx.schedule_shutdown(None);
                }
            }
        }

        // Runtime only: metadata emission omits these imports, helpers, and worker code.
        use boomerang_runtime::{
            clock::{PhysicalClockError, PhysicalInstant},
            input::{InputBatch, InputErrorKind, InputObservation, InputValue},
        };

        /// Joins the application's finite, nonblocking one-shot driver on teardown.
        #[derive(Debug)]
        pub struct DriverState {
            /// Finite driver thread joined when component state leaves the runtime.
            worker: Option<std::thread::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>>>,
        }

        impl DriverState {
            /// Starts before tagged work, with source progress and clock control ready.
            pub fn new(init: DriverInit) -> Result<Self, std::io::Error> {
                let worker = std::thread::Builder::new()
                    .name("sample-driver".into())
                    .spawn(move || {
                        match init.clock.advance_to(PhysicalInstant(10_000_000)) {
                            Ok(()) => {}
                            Err(PhysicalClockError::Closed) => return Ok(()),
                            Err(error) => return Err(error.into()),
                        }
                        let selected = init.source.clock();
                        let batch = InputBatch::new(vec![InputObservation {
                            source: init.source.clone(),
                            sequence: 1,
                            domain: selected.domain(),
                            epoch: selected.epoch(),
                            acquired: PhysicalInstant(10_000_000),
                            values: vec![InputValue::new(&init.sample, 42)],
                        }]);
                        let admitted = init.source.submit(
                            batch.with_progress(vec![(init.source.clone(), PhysicalInstant(10_000_001))]),
                        );
                        match admitted {
                            Ok(_) => {}
                            Err(error) if error.kind == InputErrorKind::Disconnected => return Ok(()),
                            Err(error) => return Err(error.into()),
                        }
                        match init.clock.advance_to(PhysicalInstant(11_000_000)) {
                            Ok(()) | Err(PhysicalClockError::Closed) => {}
                            Err(error) => return Err(error.into()),
                        }
                        Ok(())
                    })?;
                Ok(Self {
                    worker: Some(worker),
                })
            }
        }

        impl Drop for DriverState {
            fn drop(&mut self) {
                if let Some(worker) = self.worker.take() {
                    match worker.join() {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => eprintln!("sample driver failed: {error}"),
                        Err(_) => eprintln!("sample driver panicked"),
                    }
                }
            }
        }
    }
}
