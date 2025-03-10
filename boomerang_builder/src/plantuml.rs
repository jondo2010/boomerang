use std::io::Write;

use slotmap::Key;

use crate::{BuilderActionKey, BuilderReactorKey, TimerSpec};

use super::{
    ActionType, BuilderError, BuilderPortKey, BuilderReactionKey, EnvBuilder, PortType,
    ReactorBuilder,
};

trait ElemId {
    fn elem_id(&self, env_builder: &EnvBuilder) -> String;
}

impl ElemId for BuilderReactionKey {
    /// Build a unique identifier for a reaction node in the PlantUML graph.
    fn elem_id(&self, env_builder: &EnvBuilder) -> String {
        let id = self.data().as_ffi() % env_builder.reaction_builders.len() as u64;
        format!("r{id}")
    }
}

impl ElemId for BuilderPortKey {
    /// Build a unique identifier for a port node in the PlantUML graph.
    fn elem_id(&self, env_builder: &EnvBuilder) -> String {
        let id = self.data().as_ffi() % env_builder.port_builders.len() as u64;
        format!("p{id}")
    }
}

impl ElemId for BuilderActionKey {
    /// Build a unique identifier for an action node in the PlantUML graph.
    fn elem_id(&self, env_builder: &EnvBuilder) -> String {
        let id = self.data().as_ffi() % env_builder.action_builders.len() as u64;
        format!("a{id}")
    }
}

impl ElemId for BuilderReactorKey {
    /// Build a unique identifier for a reactor node in the PlantUML graph.
    fn elem_id(&self, env_builder: &EnvBuilder) -> String {
        let id = self.data().as_ffi() % env_builder.reactor_builders.len() as u64;
        format!("rtr{id}")
    }
}

impl EnvBuilder {
    const BANK_EDGE: &str = "[thickness=2]";

    fn node_id(&self, key: impl ElemId) -> String {
        key.elem_id(self)
    }

    fn puml_write_ports<W: std::io::Write>(
        &self,
        reactor: &ReactorBuilder,
        buf: &mut W,
    ) -> std::io::Result<()> {
        let ports = reactor.ports.keys();
        let ports_grouped = self
            .ports_debug_grouped(ports)
            .into_iter()
            .map(|(first_key, _, _)| {
                let port = &self.port_builders[first_key];
                (port, self.node_id(first_key), port.bank_info().is_some())
            });
        for (port, port_id, is_bank) in ports_grouped {
            let port_name = port.name();
            let port_type = match port.port_type() {
                PortType::Input => "portin",
                PortType::Output => "portout",
            };

            if is_bank {
                // we assume now that this port is banked, we generate an output only for the first key
                let bank_info = port.bank_info().unwrap();
                let bank = format!("[0..{}]", bank_info.total - 1);

                writeln!(
                    buf,
                    "{port_type} \"{port_name}{bank}\" <<bank>> as {port_id}"
                )?;
            } else {
                writeln!(buf, "{port_type} \"{port_name}\" as {port_id}")?;
            }
        }
        Ok(())
    }

    fn puml_write_reaction_nodes<W: std::io::Write>(
        &self,
        reactor: &ReactorBuilder,
        buf: &mut W,
    ) -> std::io::Result<()> {
        for (reaction_id, reaction) in reactor.reactions.keys().map(|reaction_key| {
            (
                self.node_id(reaction_key),
                &self.reaction_builders[reaction_key],
            )
        }) {
            writeln!(
                buf,
                "action \"{name}({priority})[[{{{name}}}]]\" as {id}",
                priority = reaction.priority,
                name = reaction.name,
                id = reaction_id
            )?;
        }
        Ok(())
    }

    fn puml_write_reaction_edges<W: std::io::Write>(
        &self,
        reactor: &ReactorBuilder,
        buf: &mut W,
    ) -> std::io::Result<()> {
        for (reaction_key, reaction) in reactor
            .reactions
            .keys()
            .map(|reaction_key| (reaction_key, &self.reaction_builders[reaction_key]))
        {
            for (port_key, last_port_key, _) in
                self.ports_debug_grouped(reaction.port_relations.keys())
            {
                let reaction_id = self.node_id(reaction_key);
                let port_id = self.node_id(port_key);
                let props = if last_port_key.is_some() {
                    Self::BANK_EDGE
                } else {
                    ""
                };

                let trigger_mode = reaction.port_relations[port_key];
                match trigger_mode {
                    crate::TriggerMode::TriggersOnly => {
                        writeln!(buf, "{port_id} .{props}-> {reaction_id} : trig_only")?;
                    }
                    crate::TriggerMode::TriggersAndUses => {
                        writeln!(buf, "{port_id} .{props}-> {reaction_id} : trig")?;
                    }
                    crate::TriggerMode::TriggersAndEffects => {
                        writeln!(buf, "{port_id} .{props}-> {reaction_id} : trig")?;
                        writeln!(buf, "{reaction_id} .{props}-> {port_id} : eff")?;
                    }
                    crate::TriggerMode::UsesOnly => {
                        writeln!(buf, "{port_id} .{props}-> {reaction_id} : use_only")?;
                    }
                    crate::TriggerMode::EffectsOnly => {
                        writeln!(buf, "{reaction_id} .{props}-> {port_id} : eff")?;
                    }
                }
            }
        }
        Ok(())
    }

    fn puml_write_action_nodes<W: std::io::Write>(
        &self,
        reactor: &ReactorBuilder,
        buf: &mut W,
    ) -> std::io::Result<()> {
        for (action_key, action) in reactor
            .actions
            .keys()
            .map(|action_key| (action_key, &self.action_builders[action_key]))
        {
            let action_id = self.node_id(action_key);
            let (xlabel, tooltip): (String, String) = match action.r#type() {
                ActionType::Timer(timer_spec) if timer_spec == &TimerSpec::STARTUP => {
                    ("\u{2600}".into(), "Startup".into())
                }
                ActionType::Timer(TimerSpec { period, offset }) => {
                    let label = "\u{23f2}".into();
                    let tt = if offset.unwrap_or_default().is_zero() {
                        "Timer".into()
                    } else {
                        format!(
                            "{} ({:?}, {:?})",
                            action.name(),
                            offset.unwrap_or_default(),
                            period.unwrap_or_default()
                        )
                    };
                    (label, tt)
                }
                ActionType::Standard {
                    is_logical,
                    min_delay,
                    ..
                } => {
                    let xlabel = if *is_logical {
                        format!("L({})", action.name())
                    } else {
                        format!("P({})", action.name())
                    };
                    (
                        xlabel,
                        format!("{} ({:?})", action.name(), min_delay.unwrap_or_default()),
                    )
                }
                ActionType::Shutdown => ("\u{263d}".into(), "Shutdown".into()),
            };

            // Only show if this action shows up in any Reaction's `action_relations`
            if self
                .reaction_builders
                .iter()
                .any(|(_, reaction)| reaction.action_relations.contains_key(action_key))
            {
                writeln!(
                    buf,
                    "hexagon \"{label}[[{{{tooltip}}}]]\" as {id}",
                    label = xlabel,
                    tooltip = tooltip,
                    id = action_id
                )?;
            }
        }
        Ok(())
    }

    fn puml_write_action_edges<W: std::io::Write>(
        &self,
        reactor: &ReactorBuilder,
        buf: &mut W,
    ) -> std::io::Result<()> {
        for (reaction_key, reaction) in reactor
            .reactions
            .keys()
            .map(|reaction_key| (reaction_key, &self.reaction_builders[reaction_key]))
        {
            let reaction_id = self.node_id(reaction_key);
            for (action_key, trigger_mode) in reaction.action_relations.iter() {
                let action_id = self.node_id(action_key);
                match trigger_mode {
                    crate::TriggerMode::TriggersOnly | crate::TriggerMode::TriggersAndUses => {
                        writeln!(buf, "{action_id} .-> {reaction_id} : trig")?;
                    }
                    crate::TriggerMode::TriggersAndEffects => {
                        writeln!(buf, "{action_id} .-> {reaction_id} : trig")?;
                        writeln!(buf, "{reaction_id} .-> {action_id} : sched")?;
                    }
                    crate::TriggerMode::UsesOnly => {
                        writeln!(buf, "{action_id} .-> {reaction_id} : use")?;
                    }
                    crate::TriggerMode::EffectsOnly => {
                        writeln!(buf, "{reaction_id} .-> {action_id} : sched")?;
                    }
                }
            }
        }

        Ok(())
    }

    fn build_port_bindings<W: std::io::Write>(&self, buf: &mut W) -> std::io::Result<()> {
        //TODO: this is a naive implementation, we should group by bank
        for connection in &self.connection_builders {
            let source_id = self.node_id(connection.source_key());
            let target_id = self.node_id(connection.target_key());
            writeln!(buf, "{source_id} --> {target_id}")?;
        }
        Ok(())
    }

    /// Build a PlantUML representation of the entire Reactor environment. This creates a top-level view
    /// of all defined Reactors and any nested children.
    pub fn create_plantuml_graph(&self) -> Result<String, BuilderError> {
        let graph = self.build_reactor_graph_grouped();
        let ordered_reactors = petgraph::algo::toposort(&graph, None)
            .map_err(|e| BuilderError::ReactorGraphCycle { what: e.node_id() })?;
        let start = *ordered_reactors.first().unwrap();

        const PREAMBLE: &str = r#"
left to right direction
!theme sandstone
skinparam componentStyle rectangle
skinparam shadowing<<bank>> true
skinparam arrowThickness 1
<style>
    .bank {
        lineThickness 2
        fontStyle bold
    }
    component {
    }
    hexagon {
        'LineColor LightCyan
    }
    action {
        'LineColor LightYellow
    }
</style>"#;

        let mut buf = Vec::new();
        let mut edge_buf = Vec::new();
        writeln!(&mut buf, "@startuml{PREAMBLE}").unwrap();

        petgraph::visit::depth_first_search(&graph, Some(start), |event| match event {
            petgraph::visit::DfsEvent::Discover(reactor_key, _) => {
                let reactor = &self.reactor_builders[reactor_key];
                let bank = reactor
                    .bank_info()
                    .map(|bi| format!("[0..{}]", bi.total - 1));

                let enclave = if reactor.is_enclave { "📦 " } else { "" };
                let stereotype = if bank.is_some() { " <<bank>> " } else { "" };

                let reactor_id = self.node_id(reactor_key);
                writeln!(
                    &mut buf,
                    "component {reactor_id} as \"{enclave}{name}{bank}\"{stereotype}{{",
                    name = reactor.name(),
                    bank = bank.unwrap_or_default(),
                )
                .unwrap();

                self.puml_write_ports(reactor, &mut buf).unwrap();
                self.puml_write_reaction_nodes(reactor, &mut buf).unwrap();
                self.puml_write_action_nodes(reactor, &mut buf).unwrap();
                self.puml_write_reaction_edges(reactor, &mut edge_buf)
                    .unwrap();
                self.puml_write_action_edges(reactor, &mut edge_buf)
                    .unwrap();
            }
            petgraph::visit::DfsEvent::Finish(_, _) => {
                writeln!(&mut buf, "}}").unwrap();
            }
            _ => {}
        });

        //TODO: fix or remove
        //let reaction_graph = self.build_reaction_graph();
        //for (r1, r2, _) in reaction_graph.all_edges() {
        //    let r1_id = self.node_id(r1);
        //    let r2_id = self.node_id(r2);
        //    writeln!(&mut buf, "{r1_id} -> {r2_id}").unwrap();
        //}

        buf.write_all(&edge_buf).unwrap();
        self.build_port_bindings(&mut buf).unwrap();

        writeln!(&mut buf, "@enduml").unwrap();
        Ok(String::from_utf8(buf).unwrap())
    }
}
