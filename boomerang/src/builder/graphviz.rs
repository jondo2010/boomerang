use std::collections::HashMap;

use super::{ActionType, BuilderError, EnvBuilder, PortType, ReactorBuilder};
use crate::runtime;
use itertools::Itertools;
use slotmap::Key;

pub fn build_reaction_graph<S: runtime::SchedulerPoint>(
    env_builder: &EnvBuilder<S>,
) -> Result<String, BuilderError> {
    let reaction_graph = env_builder.get_reaction_graph();
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
        output.push(format!("subgraph cluster{} {{", level));
        output.push(format!("  label=\"level{}\";", level));

        for &key in reactions.iter() {
            let reaction = &env_builder.reaction_builders[key];
            let reaction_id = key.data().as_ffi() % env_builder.reaction_builders.len() as u64;
            output.push(format!(
                "  r{} [label=\"{}\";shape=cds;color=3];",
                reaction_id, reaction.name
            ));
        }

        output.push(format!("}}"));
    }

    for (from, to, _) in reaction_graph.all_edges() {
        let from_id = from.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        let to_id = to.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        output.push(format!("  r{} -> r{};", from_id, to_id));
    }

    output.push(format!("}}\n"));
    Ok(output.join("\n"))
}

/// Build a GraphViz representation of the entire Reactor
pub fn build<S: runtime::SchedulerPoint>(
    env_builder: &EnvBuilder<S>,
) -> Result<String, BuilderError> {
    let graph = env_builder.build_reactor_graph();
    let ordered_reactors = petgraph::algo::toposort(&graph, None)
        .map_err(|e| BuilderError::ReactorGraphCycle { what: e.node_id() })?;
    let start = *ordered_reactors.first().unwrap();

    let mut output = vec![
        "digraph G {".to_owned(),
        format!(
            "  rankdir=\"LR\";labeljust=\"l\";colorscheme=\"{}\";bgcolor=\"{}\";",
            "greys8", "1:2"
        ),
        format!("  node [style=filled;colorscheme=\"{}\"];", "accent8"),
    ];

    petgraph::visit::depth_first_search(&graph, Some(start), |event| match event {
        petgraph::visit::DfsEvent::Discover(key, _) => {
            let reactor = &env_builder.reactors[key];
            let reactor_id = key.data().as_ffi() % env_builder.reactors.len() as u64;

            output.push(format!("subgraph cluster{} {{", reactor_id));
            output.push(format!("  label=\"{}\";", reactor.name));
            output.push(format!("  style=\"rounded\"; node [shape=record];"));

            build_ports(env_builder, reactor, reactor_id, &mut output);
            build_reactions(env_builder, reactor, reactor_id, &mut output);
            build_actions(env_builder, reactor, &mut output);
        }
        petgraph::visit::DfsEvent::Finish(_, _) => {
            output.push(format!("}}"));
        }
        _ => {}
    });

    build_port_bindings(env_builder, &mut output);
    output.push(format!("}}\n"));
    Ok(output.join("\n"))
}

fn build_ports<S>(
    env_builder: &EnvBuilder<S>,
    reactor: &ReactorBuilder,
    reactor_id: u64,
    output: &mut Vec<String>,
) {
    let (inputs, outputs): (Vec<_>, Vec<_>) = reactor.ports.keys().partition_map(|key| {
        let port_id = key.data().as_ffi() % env_builder.ports.len() as u64;
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

fn build_reactions<S>(
    env_builder: &EnvBuilder<S>,
    reactor: &ReactorBuilder,
    reactor_id: u64,
    output: &mut Vec<String>,
) {
    for (reaction_key, reaction) in reactor
        .reactions
        .keys()
        .map(|reaction_key| (reaction_key, &env_builder.reaction_builders[reaction_key]))
    {
        let reaction_id = reaction_key.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        output.push(format!(
            "  r{} [label=\"{}\";shape=cds;color=3];",
            reaction_id, reaction.name
        ));
        // output.push(format!(
        //    "  inputs{} -> r{} -> outputs{} [style=invis];",
        //    reactor_id, reaction_id, reactor_id
        //));
        for port_key in reaction.deps.keys() {
            let port_id = port_key.data().as_ffi() % env_builder.ports.len() as u64;
            output.push(format!(
                "  inputs{}:p{}:e -> r{}:w;",
                reactor_id, port_id, reaction_id
            ));
        }
        for port_key in reaction.antideps.keys() {
            let port_id = port_key.data().as_ffi() % env_builder.ports.len() as u64;
            output.push(format!(
                "  r{}:e -> outputs{}:p{}:w;",
                reaction_id, reactor_id, port_id
            ));
        }
    }
}

fn build_actions<S>(
    env_builder: &EnvBuilder<S>,
    reactor: &ReactorBuilder,
    output: &mut Vec<String>,
) {
    for action_key in reactor.actions.keys() {
        let action = &env_builder.action_builders[action_key];
        let action_id = action_key.data().as_ffi() % env_builder.actions.len() as u64;

        let xlabel = match action.get_type() {
            ActionType::Timer { period, offset } => {
                if offset.is_zero() {
                    format!("⏲(startup)")
                } else {
                    format!("⏲({} ms, {} ms)", offset.as_millis(), period.as_millis())
                }
            }
            ActionType::Logical { min_delay } => {
                format!("L({} ms)", min_delay.unwrap_or_default().as_millis())
            }
            ActionType::Shutdown => {
                format!("Shutdown")
            }
        };

        output.push(format!(
            "  a{} [label=\"{}\";xlabel=\"{}\"shape=diamond;color=4];",
            action_id,
            action.get_name(),
            xlabel,
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

fn build_port_bindings<S>(env_builder: &EnvBuilder<S>, output: &mut Vec<String>) {
    env_builder
        .port_builders
        .iter()
        .flat_map(|(port_key, port)| {
            port.get_outward_bindings().map(move |binding_key| {
                let port_id = port_key.data().as_ffi() % env_builder.ports.len() as u64;
                let port_reactor_id =
                    port.get_reactor_key().data().as_ffi() % env_builder.reactors.len() as u64;

                let binding = &env_builder.port_builders[binding_key];
                let binding_id = binding_key.data().as_ffi() % env_builder.ports.len() as u64;
                let binding_reactor_id =
                    binding.get_reactor_key().data().as_ffi() % env_builder.reactors.len() as u64;

                let from = match port.get_port_type() {
                    PortType::Input => format!("inputs{}:p{}:e", port_reactor_id, port_id),
                    PortType::Output => format!("outputs{}:p{}:e", port_reactor_id, port_id),
                };

                let to = match binding.get_port_type() {
                    PortType::Input => format!("inputs{}:p{}:w", binding_reactor_id, binding_id),
                    PortType::Output => format!("outputs{}:p{}:w", binding_reactor_id, binding_id),
                };

                (from, to)
            })
        })
        .for_each(|(from, to)| {
            output.push(format!("  {} -> {};", from, to));
        });
}
