use boomerang::prelude::*;

#[cfg(not(any(
    feature = "__boomerang_descriptor",
    feature = "__boomerang_payload"
)))]
fn hosted_state_constructor() -> usize {
    7
}

#[cfg(not(any(
    feature = "__boomerang_descriptor",
    feature = "__boomerang_payload"
)))]
fn hosted_reaction_payload() {}

#[reactor]
pub fn Hosted(
    #[output] output: usize,
    #[state(default = hosted_state_constructor())] count: usize,
) -> impl Reactor {
    reaction! {
        (startup) -> output {
            hosted_reaction_payload();
            *output = Some(state.count);
        }
    }
}

#[cfg(feature = "__boomerang_descriptor")]
pub mod __boomerang {
    pub const HOSTED_ONLY: () = ();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_reactor_and_state_constructor_remain_available() {
        let _reactor = Hosted();
        assert_eq!(HostedState::default().count, 7);
    }
}
