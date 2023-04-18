//! Methods for constructing Graphviz graphs representing the `EnvBuilder` useful for debugging and
//! understand the Reactor graph.

use super::{
    ActionType, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey,
    BuilderReactorKey, EnvBuilder, PortType, ReactorBuilder,
};

use itertools::Itertools;
use slotmap::Key;
use std::collections::HashMap;

/// Configuration for the graphviz generation
pub struct Config {
    pub show_reaction_graph_edges: bool,
    pub hide_orphaned_actions: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_reaction_graph_edges: false,
            hide_orphaned_actions: true,
        }
    }
}

impl Config {
    /// Create a new `Config` with the default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the edges between the reactions in the graph.
    pub fn show_reaction_graph_edges(mut self, value: bool) -> Self {
        self.show_reaction_graph_edges = value;
        self
    }

    /// Hide actions that are not connected to any reactions.
    pub fn hide_orphaned_actions(mut self, value: bool) -> Self {
        self.hide_orphaned_actions = value;
        self
    }
}

fn action_id(key: BuilderActionKey, reactor_id: u64, reactor: &ReactorBuilder) -> u64 {
    1000 * reactor_id + (key.data().as_ffi() % (reactor.actions.len() as u64 + 1))
}

fn reaction_id(key: BuilderReactionKey, env: &EnvBuilder) -> u64 {
    key.data().as_ffi() % (env.reaction_builders.len() as u64 + 1)
}

fn port_id(key: BuilderPortKey, env: &EnvBuilder) -> u64 {
    key.data().as_ffi() % (env.port_builders.len() as u64 + 1)
}

fn reactor_id(key: BuilderReactorKey, env: &EnvBuilder) -> u64 {
    key.data().as_ffi() % (env.reactor_builders.len() as u64 + 1)
}

/// Build the node name string for a Port
fn port_node_name(env_builder: &EnvBuilder, port_key: BuilderPortKey) -> String {
    let port = &env_builder.port_builders[port_key];
    let port_id = port_id(port_key, env_builder);
    let port_reactor_id = reactor_id(port.get_reactor_key(), env_builder);
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
        let port_id = port_id(key, env_builder);
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
        let reaction_id = reaction_id(reaction_key, env_builder);
        output.push(format!(
            "  r{} [label=\"{} ({})\";shape=cds;color=3];",
            reaction_id, reaction.name, reaction.priority
        ));
        // output.push(format!(
        //    "  inputs{} -> r{} -> outputs{} [style=invis];",
        //    reactor_id, reaction_id, reactor_id
        //));
        for port_key in reaction.input_ports.keys() {
            let port_node = port_node_name(env_builder, port_key);
            output.push(format!("  {}:e -> r{}:w;", port_node, reaction_id));
        }
        for port_key in reaction.output_ports.keys() {
            let port_node = port_node_name(env_builder, port_key);
            output.push(format!("  r{}:e -> {}:w;", reaction_id, port_node));
        }
    }
}

fn build_actions(
    env_builder: &EnvBuilder,
    reactor: &ReactorBuilder,
    reactor_id: u64,
    output: &mut Vec<String>,
    hide_orphaned_actions: bool,
) {
    for (action_key, action) in reactor.actions.iter() {
        let action_id = action_id(action_key, reactor_id, reactor);

        let xlabel = match action.get_type() {
            ActionType::Timer { period, offset } => {
                format!("⏲({offset:?}, {period:?})")
            }
            ActionType::Logical { min_delay } => {
                format!("L({:?})", min_delay.unwrap_or_default())
            }
            ActionType::Physical { min_delay } => {
                format!("P({:?})", min_delay.unwrap_or_default())
            }
            ActionType::Startup => "".into(),
            ActionType::Shutdown => "".into(),
        };

        let skip_action =
            hide_orphaned_actions && action.triggers.is_empty() && action.schedulers.is_empty();

        if !skip_action {
            output.push(format!(
                "  a{action_id} [label=\"{}\"; xlabel=\"{xlabel}\"shape=oval;color=4];",
                action.get_name(),
            ));

            for reaction_key in action.triggers.keys() {
                let reaction_id = reaction_id(reaction_key, env_builder);
                output.push(format!("  a{action_id}:e -> r{reaction_id}:w;"));
            }

            for reaction_key in action.schedulers.keys() {
                let reaction_id = reaction_id(reaction_key, env_builder);
                output.push(format!(
                    "  r{reaction_id}:e -> a{action_id}:w [style=dashed];",
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
///
/// # Arguments
/// show_reaction_graph_edges: If true, show the edges between reactions in the graph. This is useful
/// for debugging cycles in the Reactor graph.
pub fn create_full_graph(env_builder: &EnvBuilder, config: Config) -> Result<String, BuilderError> {
    let graph = env_builder.build_reactor_graph();
    let ordered_reactors = petgraph::algo::toposort(&graph, None)
        .map_err(|e| BuilderError::ReactorGraphCycle { what: e.node_id() })?;

    let mut output = vec![
        "digraph G {".to_owned(),
        format!(
            "  rankdir=\"LR\";labeljust=\"l\";colorscheme=\"{}\";bgcolor=\"{}\";",
            "greys8", "white"
        ),
        format!("  node [style=filled;colorscheme=\"{}\"];", "accent8"),
    ];

    petgraph::visit::depth_first_search(&graph, ordered_reactors.iter().copied(), |event| {
        match event {
            petgraph::visit::DfsEvent::Discover(key, _) => {
                let reactor = &env_builder.reactor_builders[key];
                let reactor_id = reactor_id(key, env_builder);

                output.push(format!("subgraph cluster{} {{", reactor_id));
                output.push(format!(
                    "  label=\"{} '{}'\";",
                    reactor.type_name(),
                    reactor.get_name()
                ));
                output.push("  style=\"rounded\"; node [shape=record];".into());

                build_ports(env_builder, reactor, reactor_id, &mut output);
                build_reactions(env_builder, reactor, &mut output);
                build_actions(
                    env_builder,
                    reactor,
                    reactor_id,
                    &mut output,
                    config.hide_orphaned_actions,
                );
            }
            petgraph::visit::DfsEvent::Finish(_, _) => {
                output.push("}".into());
            }
            _ => {}
        }
    });

    if config.show_reaction_graph_edges {
        let reaction_graph = env_builder.build_reaction_graph();
        for (r1, r2, _) in reaction_graph.all_edges() {
            let r1_id = reaction_id(r1, env_builder);
            let r2_id = reaction_id(r2, env_builder);
            output.push(format!(
                "r{r1_id} -> r{r2_id} [style=dashed;color=red;constraint=false];"
            ));
        }
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
        output.push(format!("subgraph cluster{} {{", level));
        output.push(format!("  label=\"level{}\";", level));

        for &key in reactions.iter() {
            let _reaction = &env_builder.reaction_builders[key];
            let reaction_id = reaction_id(key, env_builder);
            output.push(format!(
                "  r{} [label=\"{}\";shape=cds;color=3];",
                reaction_id,
                env_builder.reaction_fqn(key).unwrap()
            ));
        }

        output.push("}".into());
    }

    for (from, to, _) in reaction_graph.all_edges() {
        let from_id = reaction_id(from, env_builder);
        let to_id = reaction_id(to, env_builder);
        output.push(format!("  r{} -> r{};", from_id, to_id));
    }

    output.push("}\n".into());
    Ok(output.join("\n"))
}
