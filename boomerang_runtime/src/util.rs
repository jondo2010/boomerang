use crate::{DepInfo, Env};

/// Utility function to check consistency between Env and DepInfo structs.
pub fn assert_consistency(env: &Env, dep_info: &DepInfo) {
    for port_key in env.ports.keys() {
        assert!(
            dep_info.port_triggers.contains_key(port_key),
            "PortKey {:?} missing in dep_info.port_triggers!",
            port_key
        );
    }

    for action_key in env.actions.keys() {
        assert!(
            dep_info.action_triggers.contains_key(action_key),
            "ActionKey {:?} missing in dep_info.action_triggers!",
            action_key
        );
    }

    for reaction_key in env.reactions.keys() {
        assert!(
            dep_info.reaction_levels.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_levels!",
            reaction_key
        );
        assert!(
            dep_info.reaction_inputs.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_inputs!",
            reaction_key
        );
        assert!(
            dep_info.reaction_outputs.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_outputs!",
            reaction_key
        );
        assert!(
            dep_info.reaction_trig_actions.contains_key(reaction_key),
            "ReactionKey {:?} missing in dep_info.reaction_trig_actions!",
            reaction_key
        );
    }
}

/// Print debug info about an Env/DepInfo pair.
pub fn print_debug_info(env: &Env, dep_info: &DepInfo) {
    // Which Reactions are triggered by each Action
    for (action_key, action) in env.actions.iter() {
        let mut action_pairs: Vec<_> = dep_info.triggered_by_action(action_key).collect();
        if action_pairs.len() > 0 {
            action_pairs.sort_by_key(|(level, _)| *level);
            println!("Action {:?} ({}) triggers:", action_key, action.get_name());
            for (level, reaction_key) in action_pairs {
                println!(
                    "  {level}: {:?} ({})",
                    reaction_key,
                    env.reactions[reaction_key].get_name()
                );
            }
        }
    }

    // Which Reactions are triggered by each port
    for (port_key, port) in env.ports.iter() {
        let mut port_pairs: Vec<_> = dep_info.triggered_by_port(port_key).collect();
        if port_pairs.len() > 0 {
            port_pairs.sort_by_key(|(level, _)| *level);
            println!("{port} triggers:");
            for (level, reaction_key) in port_pairs {
                println!(
                    "  {level}: {:?} ({})",
                    reaction_key,
                    env.reactions[reaction_key].get_name()
                );
            }
        }
    }

    for (reaction_key, reaction) in env.reactions.iter() {
        println!("{reaction:?}");
        if !dep_info.reaction_inputs[reaction_key].is_empty() {
            println!("  inputs:");
            for &port_key in dep_info.reaction_inputs[reaction_key].iter() {
                println!("   . {}", env.ports[port_key]);
            }
        }
        if !dep_info.reaction_outputs[reaction_key].is_empty() {
            println!("  outputs:");
            for &port_key in dep_info.reaction_outputs[reaction_key].iter() {
                println!("   . {}", env.ports[port_key]);
            }
        }
        if !dep_info.reaction_trig_actions[reaction_key].is_empty() {
            println!("  triggers:");
            for &action_key in dep_info.reaction_trig_actions[reaction_key].iter() {
                println!("   . {}", env.actions[action_key]);
            }
        }
        if !dep_info.reaction_sched_actions[reaction_key].is_empty() {
            println!("  schedulable actions:");
            for &action_key in dep_info.reaction_sched_actions[reaction_key].iter() {
                println!("   . {}", env.actions[action_key]);
            }
        }
    }
}
