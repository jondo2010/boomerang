//! Methods for constructing Graphviz graphs representing the [`EnvBuilder`] useful for debugging and
//! understand the Reactor graph.

use super::{
    ActionType, BuilderError, BuilderPortKey, EnvBuilder, PortType, ReactorBuilder, TimerSpec,
};

use itertools::Itertools;
use slotmap::Key;
use std::collections::HashMap;

/// Build the node name string for a Port
fn port_node_name(env_builder: &EnvBuilder, port_key: BuilderPortKey) -> String {
    let port = &env_builder.port_builders[port_key];
    let port_id = port_key.data().as_ffi() % env_builder.port_builders.len() as u64;
    let port_reactor_id =
        port.get_reactor_key().data().as_ffi() % env_builder.reactor_builders.len() as u64;
    match port.get_port_type() {
        PortType::Input => format!("inputs{}:p{}", port_reactor_id, port_id),
        PortType::Output => format!("outputs{}:p{}", port_reactor_id, port_id),
    }
}

fn build_ports(
    env_builder: &EnvBuilder,
    reactor: &ReactorBuilder,
    reactor_id: u64,
    output: &mut Vec<String>,
) {
    let (inputs, outputs): (Vec<_>, Vec<_>) = reactor.ports.keys().partition_map(|key| {
        let port_id = key.data().as_ffi() % env_builder.port_builders.len() as u64;
        let port = &env_builder.port_builders[key];
        let s = format!("<p{}> {}", port_id, port.get_name());

        match port.get_port_type() {
            PortType::Input => itertools::Either::Left(s),
            PortType::Output => itertools::Either::Right(s),
        }
    });
    if !inputs.is_empty() {
        output.push(format!(
            "  inputs{} [label=\"{}\";color=1];",
            reactor_id,
            inputs.join("|")
        ));
    }
    if !outputs.is_empty() {
        output.push(format!(
            "  outputs{} [label=\"{}\";color=2];",
            reactor_id,
            outputs.join("|")
        ));
    }
}

fn build_reactions(env_builder: &EnvBuilder, reactor: &ReactorBuilder, output: &mut Vec<String>) {
    for (reaction_key, reaction) in reactor
        .reactions
        .keys()
        .map(|reaction_key| (reaction_key, &env_builder.reaction_builders[reaction_key]))
    {
        let reaction_id = reaction_key.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        output.push(format!(
            "  r{} [label=\"{} ({})\";shape=cds;color=3];",
            reaction_id, reaction.name, reaction.priority
        ));
        // output.push(format!(
        //    "  inputs{} -> r{} -> outputs{} [style=invis];",
        //    reactor_id, reaction_id, reactor_id
        //));
        for port_key in reaction.trigger_ports.keys() {
            let port_node = port_node_name(env_builder, port_key);
            output.push(format!("  {}:e -> r{}:w;", port_node, reaction_id));
        }
        for port_key in reaction.effect_ports.keys() {
            let port_node = port_node_name(env_builder, port_key);
            output.push(format!("  r{}:e -> {}:w;", reaction_id, port_node));
        }
    }
}

fn build_actions(env_builder: &EnvBuilder, reactor: &ReactorBuilder, output: &mut Vec<String>) {
    for action_key in reactor.actions.keys() {
        let action = &env_builder.action_builders[action_key];
        let action_id = action_key.data().as_ffi() % reactor.actions.len() as u64;

        let xlabel = match action.get_type() {
            ActionType::Timer(TimerSpec { period, offset }) => {
                if offset.unwrap_or_default().is_zero() {
                    "⏲ (startup)".into()
                } else {
                    format!(
                        "⏲ ({} ms, {} ms)",
                        offset.unwrap_or_default().as_millis(),
                        period.unwrap_or_default().as_millis()
                    )
                }
            }
            ActionType::Logical { min_delay } => {
                format!("L({} ms)", min_delay.unwrap_or_default().as_millis())
            }
            ActionType::Physical { min_delay } => {
                format!("P({} ms)", min_delay.unwrap_or_default().as_millis())
            }
            ActionType::Startup => "Startup".into(),
            ActionType::Shutdown => "Shutdown".into(),
        };

        if !action.triggers.is_empty() || !action.schedulers.is_empty() {
            output.push(format!(
                "  a{action_id} [label=\"{}\"; xlabel=\"{xlabel}\"shape=diamond;color=4];",
                action.get_name(),
            ));

            for reaction_key in action.triggers.keys() {
                let reaction_id =
                    reaction_key.data().as_ffi() % env_builder.reaction_builders.len() as u64;
                output.push(format!("  a{}:e -> r{}:w;", action_id, reaction_id));
            }

            for reaction_key in action.schedulers.keys() {
                let reaction_id =
                    reaction_key.data().as_ffi() % env_builder.reaction_builders.len() as u64;

                output.push(format!(
                    "  r{}:e -> a{}:w [style=dashed];",
                    reaction_id, action_id
                ));
            }
        }
    }
}

fn build_port_bindings(env_builder: &EnvBuilder, output: &mut Vec<String>) {
    env_builder
        .port_builders
        .iter()
        .flat_map(|(port_key, port)| {
            port.get_outward_bindings().map(move |binding_key| {
                let from = port_node_name(env_builder, port_key);
                let to = port_node_name(env_builder, binding_key);
                (from, to)
            })
        })
        .for_each(|(from, to)| {
            output.push(format!("  {}:e -> {}:w;", from, to));
        });
}

/// Build a GraphViz representation of the entire Reactor environment. This creates a top-level view
/// of all defined Reactors and any nested children.
pub fn create_full_graph(env_builder: &EnvBuilder) -> Result<String, BuilderError> {
    let graph = env_builder.build_reactor_graph();
    let ordered_reactors = petgraph::algo::toposort(&graph, None)
        .map_err(|e| BuilderError::ReactorGraphCycle { what: e.node_id() })?;
    let start = *ordered_reactors.first().unwrap();

    let mut output = vec![
        "digraph G {".to_owned(),
        format!(
            "  rankdir=\"LR\";labeljust=\"l\";colorscheme=\"{}\";bgcolor=\"{}\";",
            "greys8", "white"
        ),
        format!("  node [style=filled;colorscheme=\"{}\"];", "accent8"),
    ];

    petgraph::visit::depth_first_search(&graph, Some(start), |event| match event {
        petgraph::visit::DfsEvent::Discover(key, _) => {
            let reactor = &env_builder.reactor_builders[key];
            let reactor_id = key.data().as_ffi() % env_builder.reactor_builders.len() as u64;

            output.push(format!("subgraph cluster{} {{", reactor_id));
            output.push(format!(
                "  label=\"{} '{}'\";",
                reactor.type_name(),
                reactor.get_name()
            ));
            output.push("  style=\"rounded\"; node [shape=record];".into());

            build_ports(env_builder, reactor, reactor_id, &mut output);
            build_reactions(env_builder, reactor, &mut output);
            build_actions(env_builder, reactor, &mut output);
        }
        petgraph::visit::DfsEvent::Finish(_, _) => {
            output.push("}".into());
        }
        _ => {}
    });

    let reaction_graph = env_builder.build_reaction_graph();
    for (r1, r2, _) in reaction_graph.all_edges() {
        let r1_id = r1.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        let r2_id = r2.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        output.push(format!(
            "r{r1_id} -> r{r2_id} [style=dashed;color=red;constraint=false];"
        ));
    }

    build_port_bindings(env_builder, &mut output);
    output.push("}\n".into());
    Ok(output.join("\n"))
}

/// Build a GraphViz representation of the "Reaction Graph", where the nodes are Reactions, and the
/// edges are dependencies between Reactions. Reactions are clustered into their "Execution Level".
/// Reactions within the same level can be scheduled in parallel.
pub fn create_reaction_graph(env_builder: &EnvBuilder) -> Result<String, BuilderError> {
    let reaction_graph = env_builder.build_reaction_graph();
    let runtime_level_map = env_builder.build_runtime_level_map()?;

    // Cluster on level
    let mut level_runtime_map = HashMap::new();
    runtime_level_map
        .into_iter()
        .for_each(|(reaction_key, level)| {
            level_runtime_map
                .entry(level)
                .or_insert(Vec::new())
                .push(reaction_key)
        });

    // Add all nodes
    let mut output = vec!["digraph G {".to_owned()];

    for (level, reactions) in level_runtime_map.iter() {
        output.push(format!("subgraph cluster{level:?} {{"));
        output.push(format!("  label=\"level{level:?}\";"));

        for &key in reactions.iter() {
            let _reaction = &env_builder.reaction_builders[key];
            let reaction_id = key.data().as_ffi() % env_builder.reaction_builders.len() as u64;
            output.push(format!(
                "  r{} [label=\"{}\";shape=cds;color=3];",
                reaction_id,
                env_builder.reaction_fqn(key).unwrap()
            ));
        }

        output.push("}".into());
    }

    for (from, to, _) in reaction_graph.all_edges() {
        let from_id = from.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        let to_id = to.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        output.push(format!("  r{} -> r{};", from_id, to_id));
    }

    output.push("}\n".into());
    Ok(output.join("\n"))
}
