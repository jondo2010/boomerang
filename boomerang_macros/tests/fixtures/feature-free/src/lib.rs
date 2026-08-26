use boomerang::prelude::*;

#[reactor]
pub fn Hosted() -> impl Reactor {}

pub fn hosted_reactor_is_available() {
    let _ = Hosted();
}
