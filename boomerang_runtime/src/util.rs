use crate::Env;

/// Print debug info about an Env/DepInfo pair.
pub fn print_debug_info(env: &Env) {
    // Which Reactions are triggered by each Action
    for (_reactor_key, reactor) in env.reactors.iter() {
        println!("Reactor \"{}\"", reactor.get_name());
        for (action_key, action) in reactor.actions.iter() {
            println!("  {action} triggers:");
            let mut action_pairs: Vec<_> = reactor.action_triggers[action_key].iter().collect();
            action_pairs.sort_by_key(|(level, _)| *level);
            for (level, reaction_key) in action_pairs.iter() {
                println!(
                    "    L{level}: {reaction_key:?} ({})",
                    env.reactions[*reaction_key].get_name()
                );
            }
        }
    }

    // Which Reactions are triggered by each port
    // for (port_key, port) in env.ports.iter() {
    //    let mut port_pairs: Vec<_> = dep_info.triggered_by_port(port_key).collect();
    //    if !port_pairs.is_empty() {
    //        port_pairs.sort_by_key(|(level, _)| *level);
    //        println!("{port} triggers:");
    //        for (level, reaction_key) in port_pairs {
    //            println!(
    //                "  {level}: {:?} ({})",
    //                reaction_key,
    //                env.reactions[reaction_key].get_name()
    //            );
    //        }
    //    }
    //}

    // for (reaction_key, reaction) in env.reactions.iter() {
    //    let reactor = &env.reactors[reaction.get_reactor_key()];
    //    println!("{reaction:?}");
    //    if !dep_info.reaction_inputs[reaction_key].is_empty() {
    //        println!("  inputs:");
    //        for &port_key in dep_info.reaction_inputs[reaction_key].iter() {
    //            println!("   - {}", env.ports[port_key]);
    //        }
    //    }
    //    if !dep_info.reaction_outputs[reaction_key].is_empty() {
    //        println!("  outputs:");
    //        for &port_key in dep_info.reaction_outputs[reaction_key].iter() {
    //            println!("   - {}", env.ports[port_key]);
    //        }
    //    }
    //}
}
