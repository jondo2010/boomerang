use boomerang::prelude::*;

#[reactor]
pub fn Dynamic() -> impl Reactor {}

pub fn dynamic_reactor_is_available() {
    let _ = Dynamic();
}
