use boomerang::prelude::*;

#[reactor]
fn Actions(
    #[logical_action] _logical_now: u32,
    #[logical_action(min_delay = 10 msec)] _logical_later: u32,
    #[physical_action] _physical_now: u16,
    #[physical_action(min_delay = 7 nsec)] _physical_later: u16,
) -> impl Reactor {
}
