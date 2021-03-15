pub mod reaction_graph {
    use boomerang_runtime::{ReactionKey, SchedulerPoint};
    use ref_cast::RefCast;
    use slotmap::Key;

    use crate::builder::EnvBuilder;

    #[derive(RefCast)]
    #[repr(transparent)]
    struct ReactionGraph<S>(EnvBuilder<S>);

    impl<'a, S: SchedulerPoint> dot::Labeller<'a> for ReactionGraph<S> {
        type Node = ReactionKey;
        type Edge = (ReactionKey, ReactionKey);

        fn graph_id(&'a self) -> dot::Id<'a> {
            dot::Id::new("ReactionGraph").unwrap()
        }

        fn node_id(&'a self, n: &ReactionKey) -> dot::Id<'a> {
            dot::Id::new(format!("N{:?}", n.data())).unwrap()
        }

        fn node_label(&'a self, n: &ReactionKey) -> dot::LabelText<'a> {
            let fqn = self.0.reaction_fqn(*n).unwrap();
            dot::LabelText::label(format!("Reaction <{}>", fqn))
        }

        fn cluster_id(&'a self, n: &ReactionKey) -> Option<dot::Id<'a>> {
            let reaction = &self.0.reaction_builders[*n];
            let key = format!("C{:?}", reaction.reactor_key.data());
            Some(dot::Id::new(key).unwrap())
        }

        fn cluster_label(&'a self, n: &ReactionKey) -> Option<dot::LabelText<'a>> {
            let reaction = &self.0.reaction_builders[*n];
            let fqn = self.0.reactor_fqn(reaction.reactor_key).unwrap();
            Some(dot::LabelText::label(format!("Reactor <{}>", fqn)))
        }
    }

    impl<'a, S: SchedulerPoint> dot::GraphWalk<'a> for ReactionGraph<S> {
        type Node = ReactionKey;
        type Edge = (ReactionKey, ReactionKey);

        fn nodes(&'a self) -> dot::Nodes<'a, ReactionKey> {
            self.0.reaction_builders.keys().collect()
        }

        fn edges(&'a self) -> dot::Edges<'a, Self::Edge> {
            self.0.reaction_dependency_edges().collect()
        }

        fn source(&'a self, edge: &Self::Edge) -> Self::Node {
            edge.0
        }

        fn target(&'a self, edge: &Self::Edge) -> Self::Node {
            edge.1
        }
    }

    pub fn render_to<S: SchedulerPoint, W: std::io::Write>(env: &EnvBuilder<S>, output: &mut W) {
        dot::render(ReactionGraph::ref_cast(env), output).unwrap()
    }
}
