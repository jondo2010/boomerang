macro_rules! declare_controller {
    ($emit:ident, $($source:tt)*) => {
        $emit! {
            pub mod controller {
                use boomerang::prelude::*;
                #[cfg(feature = "source-input")]
                use boomerang_runtime::{clock::PhysicalInstant, input::*};

                #[cfg(feature = "source-input")]
                static PEERS: std::sync::Mutex<Vec<(InputSourceHandle, SourceAction<u32>)>> =
                    std::sync::Mutex::new(Vec::new());

                /// State allocated separately for each reused component instance.
                #[derive(Debug)]
                pub struct DriverState {
                    /// Local startup counter, independent of the other instance.
                    count: u32,
                    /// Delivery count for this instance's physical action.
                    samples: u32,
                }
                impl DriverState {
                    fn new(init: DriverInit) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                        #[cfg(not(feature = "source-input"))]
                        init.send.prepare_action(&init.sample, 42, None)?.try_send()?;
                        #[cfg(feature = "source-input")]
                        {
                            let clock = init.source.clock();
                            assert!(clock.now().is_ok());
                            let mut peers = PEERS.lock().unwrap();
                            peers.push((init.source.clone(), init.sample));
                            if peers.len() == 2 {
                                let observations = peers.iter().map(|(source, action)| InputObservation {
                                    source: source.clone(), sequence: 1,
                                    acquired: PhysicalInstant(0), domain: clock.domain(), epoch: clock.epoch(),
                                    values: vec![InputValue::new(action, 42)],
                                }).collect();
                                let progress = peers.iter().map(|(source, _)|
                                    (source.clone(), PhysicalInstant(100_000_001))).collect();
                                let receipt = init.source.submit(InputBatch::new(observations).with_progress(progress))?;
                                assert_eq!(receipt.tags.len(), 2);
                                assert!(receipt.tags.iter().all(|tag| *tag == boomerang_runtime::Tag::ZERO));
                                peers.clear();
                            }
                        }
                        crate::INITIALIZED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(Self { count: 7, samples: 0 })
                    }
                }
                impl Drop for DriverState {
                    fn drop(&mut self) {
                        assert_eq!(self.count, 8, "reused state remains independent");
                        assert_eq!(self.samples, 1, "each instance receives its own sample");
                    }
                }
                #[reactor(state = DriverState, state_init = DriverState::new,
                    init_context(name = DriverInit, $($source)* actions = [sample]),
                    contract = "example.controller", contract_version = 1,
                    bounds(queue_capacity = 16, payload_bytes = 1024,
                           state_bytes = 512, scratch_bytes = 256))]
                pub fn Controller(#[input] input: u32, #[output] output: u32,
                                  #[physical_action] sample: u32) -> impl Reactor {
                    reaction! { tick(startup) -> output {
                        assert_eq!(crate::INITIALIZED.load(std::sync::atomic::Ordering::SeqCst), 2);
                        state.count += 1;
                        assert_eq!(state.count, 8);
                        *output = Some(state.count);
                        crate::REACTIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    } }
                    reaction! { connected(input) {
                        assert_eq!(*input, Some(8));
                        crate::ROUTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    } }
                    reaction! { observe(sample) {
                        assert_eq!(ctx.get_action_value(&mut sample), Some(&42));
                        #[cfg(feature = "source-input")]
                        assert_eq!(ctx.get_tag(), boomerang_runtime::Tag::ZERO);
                        state.samples += 1;
                        crate::SAMPLES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    } }
                }
            }
        }
    };
}
