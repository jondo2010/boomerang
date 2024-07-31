use std::io::Write;

use slotmap::Key;

use crate::ParentReactorBuilder;

use super::{
    ActionType, BuilderError, BuilderPortKey, BuilderReactionKey, EnvBuilder, PortType,
    ReactorBuilder,
};

macro_rules! calculate_action_id {
    ($key:expr, $env_builder:expr, $reactor_id:expr) => {
        format!(
            "a{}",
            $key.data().as_ffi() % $env_builder.reactor_builders.len() as u64 + ($reactor_id << 4)
        )
    };
}

fn reaction_node_name(env_builder: &EnvBuilder, reaction_key: BuilderReactionKey) -> String {
    let reaction = &env_builder.reaction_builders[reaction_key];
    let reaction_id = reaction_key.data().as_ffi() % env_builder.reaction_builders.len() as u64;
    let reaction_reactor_id = reaction.parent_reactor_key().unwrap().data().as_ffi()
        % env_builder.reactor_builders.len() as u64;
    format!("r{}", reaction_id + (reaction_reactor_id << 4))
}

/// Build the node name string for a Port
fn port_node_name(env_builder: &EnvBuilder, port_key: BuilderPortKey) -> String {
    let port = &env_builder.port_builders[port_key];
    let port_id = port_key.data().as_ffi() % env_builder.port_builders.len() as u64;
    let port_reactor_id = port.parent_reactor_key().unwrap().data().as_ffi()
        % env_builder.reactor_builders.len() as u64;
    format!("p{}", port_id + (port_reactor_id << 4))
}

fn build_ports<W: std::io::Write>(
    env_builder: &EnvBuilder,
    reactor: &ReactorBuilder,
    buf: &mut W,
) -> std::io::Result<()> {
    for key in reactor.ports.keys() {
        let port_id = port_node_name(env_builder, key);
        let port = &env_builder.port_builders[key];
        let port_name = port.get_name();
        let port_type = match port.get_port_type() {
            PortType::Input => "portin",
            PortType::Output => "portout",
        };
        writeln!(buf, "{port_type} {port_name} as {port_id}")?;
    }
    Ok(())
}

fn build_reactions<W: std::io::Write>(
    env_builder: &EnvBuilder,
    reactor: &ReactorBuilder,
    reactor_id: u64,
    buf: &mut W,
) -> std::io::Result<()> {
    for (reaction_key, reaction) in reactor
        .reactions
        .keys()
        .map(|reaction_key| (reaction_key, &env_builder.reaction_builders[reaction_key]))
    {
        let reaction_id = reaction_node_name(env_builder, reaction_key);
        writeln!(
            buf,
            "action \"{priority}[[{{{name}}}]]\" as {id}",
            priority = reaction.priority,
            name = reaction.name,
            id = reaction_id
        )?;

        for port_key in reaction.input_ports.keys() {
            let port_node = port_node_name(env_builder, port_key);
            writeln!(buf, "{port_node} .> {reaction_id}")?;
        }
        for port_key in reaction.output_ports.keys() {
            let port_node = port_node_name(env_builder, port_key);
            writeln!(buf, "{reaction_id} .> {port_node}")?;
        }
    }
    Ok(())
}

fn build_actions<W: std::io::Write>(
    env_builder: &EnvBuilder,
    reactor: &ReactorBuilder,
    reactor_id: u64,
    buf: &mut W,
) -> std::io::Result<()> {
    for (action_key, action) in reactor.actions.iter() {
        let action_id = calculate_action_id!(action_key, env_builder, reactor_id);

        let (xlabel, tooltip): (String, String) = match action.get_type() {
            ActionType::Timer { period, offset } => {
                let label = "\u{23f2}".into();
                let tt = if offset.is_zero() {
                    "Startup".into()
                } else {
                    format!(
                        "{} ({} ms, {} ms)",
                        action.get_name(),
                        offset.as_millis(),
                        period.as_millis()
                    )
                };
                (label, tt)
            }
            ActionType::Logical { min_delay } => (
                "L".into(),
                format!(
                    "{} ({} ms)",
                    action.get_name(),
                    min_delay.unwrap_or_default().as_millis()
                ),
            ),
            ActionType::Physical { min_delay } => (
                "P".into(),
                format!(
                    "{} ({} ms)",
                    action.get_name(),
                    min_delay.unwrap_or_default().as_millis()
                ),
            ),
            ActionType::Startup => ("\u{2600}".into(), "Startup".into()),
            ActionType::Shutdown => ("\u{263d}".into(), "Shutdown".into()),
        };

        if !action.triggers.is_empty() || !action.schedulers.is_empty() {
            writeln!(
                buf,
                "hexagon \"{label}[[{{{tooltip}}}]]\" as {id}",
                label = xlabel,
                tooltip = tooltip,
                id = action_id
            )?;

            for reaction_key in action.triggers.keys() {
                let reaction_id = reaction_node_name(env_builder, reaction_key);
                writeln!(buf, "{action_id} .> {reaction_id}")?;
            }

            for reaction_key in action.schedulers.keys() {
                let reaction_id = reaction_node_name(env_builder, reaction_key);
                writeln!(buf, "{reaction_id} .> {action_id}")?;
            }
        }
    }
    Ok(())
}

fn build_port_bindings<W: std::io::Write>(
    env_builder: &EnvBuilder,
    buf: &mut W,
) -> std::io::Result<()> {
    for (from, to) in env_builder
        .port_builders
        .iter()
        .flat_map(|(port_key, port)| {
            port.get_outward_bindings().map(move |binding_key| {
                let from = port_node_name(env_builder, port_key);
                let to = port_node_name(env_builder, binding_key);
                (from, to)
            })
        })
    {
        writeln!(buf, "{from} -down-> {to}")?;
    }
    Ok(())
}

/// Build a GraphViz representation of the entire Reactor environment. This creates a top-level view
/// of all defined Reactors and any nested children.
pub fn create_full_graph(env_builder: &EnvBuilder) -> Result<String, BuilderError> {
    let graph = env_builder.build_reactor_graph();
    let ordered_reactors = petgraph::algo::toposort(&graph, None)
        .map_err(|e| BuilderError::ReactorGraphCycle { what: e.node_id() })?;
    let start = *ordered_reactors.first().unwrap();

    let mut buf = Vec::new();
    writeln!(&mut buf, "@startuml\nleft to right direction").unwrap();

    petgraph::visit::depth_first_search(&graph, Some(start), |event| match event {
        petgraph::visit::DfsEvent::Discover(key, _) => {
            let reactor = &env_builder.reactor_builders[key];
            let reactor_id = key.data().as_ffi() % env_builder.reactor_builders.len() as u64;

            writeln!(
                &mut buf,
                "component {name} <<{type_name}>> {{",
                name = reactor.get_name(),
                type_name = reactor.type_name()
            )
            .unwrap();

            build_ports(env_builder, reactor, &mut buf).unwrap();
            build_reactions(env_builder, reactor, reactor_id, &mut buf).unwrap();
            build_actions(env_builder, reactor, reactor_id, &mut buf).unwrap();
        }
        petgraph::visit::DfsEvent::Finish(_, _) => {
            writeln!(&mut buf, "}}").unwrap();
        }
        _ => {}
    });

    let reaction_graph = env_builder.build_reaction_graph();
    for (r1, r2, _) in reaction_graph.all_edges() {
        let r1_id = reaction_node_name(env_builder, r1);
        let r2_id = reaction_node_name(env_builder, r2);
        //writeln!(&mut buf, "{r1_id} -> {r2_id}").unwrap();
    }

    build_port_bindings(env_builder, &mut buf).unwrap();

    writeln!(&mut buf, "@enduml").unwrap();
    Ok(String::from_utf8(buf).unwrap())
}
