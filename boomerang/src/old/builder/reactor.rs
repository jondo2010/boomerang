use super::{
    port::PortBuilder,
    reaction::{ReactionBodyBuilderFn, ReactionBuilder},
    NamedBuilder,
};
use crate::runtime;
use derive_more::Display;
use toolshed::list::List;
use toolshed::set::Set;
use toolshed::{Arena, CopyCell};

#[derive(Debug, Display, Copy, Clone)]
#[display(fmt = "ReactorBuilder({})", "self.get_fqn()")]
pub struct ReactorBuilder<'a, T>
where
    T: runtime::PortData,
{
    name: &'a str,
    // The Reactor owning this Reactor
    parent: CopyCell<Option<&'a ReactorBuilder<'a, T>>>,

    // actions
    // inputs
    // outputs
    ports: List<'a, &'a PortBuilder<'a, T>>,
    /// Reactions contained in this Reactor
    reactions: List<'a, &'a ReactionBuilder<'a, T>>,
    /// Child Reactors
    children: List<'a, &'a ReactorBuilder<'a, T>>,
}

impl<'a, T> PartialEq for ReactorBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn eq(&self, other: &Self) -> bool {
        self.name.eq(other.name)
    }
}

impl<'a, T> ReactorBuilder<'a, T>
where
    T: runtime::PortData,
{
    pub fn new(arena: &'a Arena, name: &str) -> &'a Self {
        arena.alloc(Self {
            name: arena.alloc_str(name),
            parent: CopyCell::new(None),
            ports: List::empty(),
            reactions: List::empty(),
            children: List::empty(),
        })
    }

    /// Create a new child ReactorBuilder
    pub fn new_child(&'a self, arena: &'a Arena, name: &str) -> &'a ReactorBuilder<'a, T> {
        let child = arena.alloc(Self {
            name: arena.alloc_str(name),
            parent: CopyCell::new(Some(self)),
            ports: List::empty(),
            reactions: List::empty(),
            children: List::empty(),
        });
        self.children.prepend(arena, child);
        child
    }

    /// Create a new ReactionBuilder
    pub fn new_reaction(
        &'a self,
        arena: &'a Arena,
        name: &str,
        priority: usize,
        body_builder: &'a ReactionBodyBuilderFn<T>,
    ) -> &'a ReactionBuilder<'a, T> {
        let reaction = ReactionBuilder::new(arena, name, self, priority, body_builder);
        self.reactions.prepend(arena, reaction);
        reaction
    }

    pub fn new_port(
        &'a self,
        arena: &'a Arena,
        name: &str,
        port_type: runtime::PortType,
    ) -> &'a PortBuilder<'a, T> {
        let port = PortBuilder::new(arena, name, self, port_type);
        self.ports.prepend(arena, port);
        port
    }

    /// Recursively create Reaction dependency graph
    pub fn reaction_dependency_graph(
        &'a self,
    ) -> impl Iterator<Item = (&'a ReactionBuilder<'a, T>, &'a ReactionBuilder<'a, T>)> {
        use itertools::Itertools;

        // Child reactors. Need to collect this otherwise we run into recursive return types.
        let children = self
            .children
            .iter()
            .flat_map(|child| child.reaction_dependency_graph())
            .collect::<Vec<_>>();

        // Connect all reactions this reaction depends upon
        let deps = self.reactions.iter().flat_map(|reaction| {
            reaction
                .get_dependencies()
                .iter()
                .flat_map(|port| port.follow_inward_binding().get_antidependencies())
                .map(move |dep| (*reaction, *dep))
        });

        // Connect internal reactions by priority
        let internal = self.reactions.iter().sorted().cloned().tuple_windows();

        children.into_iter().chain(deps).chain(internal)
    }
}

impl<'a, T> NamedBuilder<'a> for ReactorBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn get_name(&self) -> &str {
        self.name
    }
    fn get_fqn(&self) -> String {
        self.parent
            .get()
            .map(|parent| format!("{}.{}", parent.get_fqn(), self.name))
            .unwrap_or(self.name.to_string())
    }
}
