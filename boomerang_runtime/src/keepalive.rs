//! A simple channel for signalling a shutdown event to threads.
//!
//! Originally from https://users.rust-lang.org/t/using-arc-to-terminate-a-thread/81533/15

use std::sync::{
    atomic::{AtomicBool, Ordering::Relaxed},
    Arc,
};

#[derive(Debug)]
pub struct Sender(Arc<AtomicBool>);

#[derive(Clone, Debug)]
pub struct Receiver(Arc<AtomicBool>);

#[inline]
pub fn channel() -> (Sender, Receiver) {
    let arc1 = Arc::new(AtomicBool::new(false));
    let arc2 = arc1.clone();
    (Sender(arc1), Receiver(arc2))
}

impl Sender {
    #[inline]
    pub fn shutdown(&self) {
        self.0.store(true, Relaxed);
    }

    #[inline]
    pub fn new_receiver(&self) -> Receiver {
        Receiver(self.0.clone())
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Receiver {
    #[inline]
    pub fn is_shutdwon(&self) -> bool {
        self.0.load(Relaxed)
    }
}
