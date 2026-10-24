use boomerang::prelude::*;

fn dynamic_state_constructor() -> usize {
    7
}

fn dynamic_reaction_payload() {}

#[reactor]
pub fn Dynamic(
    #[output] output: usize,
    #[logical_action(min_delay = 10 msec)] _logical: u32,
    #[physical_action] _physical: u16,
    #[state(default = dynamic_state_constructor())] count: usize,
) -> impl Reactor {
    reaction! {
        (startup) -> output {
            dynamic_reaction_payload();
            *output = Some(state.count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_reactor_and_state_constructor_remain_available() {
        let _reactor = Dynamic();
        assert_eq!(DynamicState::default().count, 7);
    }
}
