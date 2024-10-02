use boomerang::prelude::*;

use std::time::Duration;

pub struct Timeout;

impl ::boomerang::builder::Reactor for Timeout {
    type State = Duration;
    fn build<'__builder>(
        name: &str,
        state: Self::State,
        parent: Option<::boomerang::builder::BuilderReactorKey>,
        bank_info: Option<::boomerang::runtime::BankInfo>,
        env: &'__builder mut ::boomerang::builder::EnvBuilder,
    ) -> Result<Self, ::boomerang::builder::BuilderError> {
        use ::boomerang::flatten_transposed::FlattenTransposedExt;
        let mut __builder = env.add_reactor(name, parent, bank_info, state);
        let mut __reactor = Self {};
        let _ = <ReactionStartup as ::boomerang::builder::Reaction<Self>>::build(
            stringify!(ReactionStartup),
            &__reactor,
            &mut __builder,
        )?
        .finish()?;
        Ok(__reactor)
    }
}

struct ReactionStartup;

impl ::boomerang::builder::Reaction<Timeout> for ReactionStartup {
    fn build<'builder>(
        name: &str,
        reactor: &Timeout,
        builder: &'builder mut ::boomerang::builder::ReactorBuilderState,
    ) -> Result<
        ::boomerang::builder::ReactionBuilderState<'builder>,
        ::boomerang::builder::BuilderError,
    > {
        #[allow(unused_variables)]
        fn __trigger_inner<'inner>(
            ctx: &mut ::boomerang::runtime::Context,
            state: &'inner mut dyn ::boomerang::runtime::ReactorState,
            ports: &'inner [::boomerang::runtime::PortRef<'inner>],
            ports_mut: &'inner mut [::boomerang::runtime::PortRefMut<'inner>],
            actions: &'inner mut [&'inner mut ::boomerang::runtime::Action],
        ) {
            let state /*: &mut <Timeout as ::boomerang::builder::Reactor>::State*/ = state
                .downcast_mut()
                .expect("Unable to downcast reactor state");
            <ReactionStartup as ::boomerang::builder::Trigger<Timeout>>::trigger(
                ReactionStartup {},
                ctx,
                state,
            );
        }
        let __startup_action = builder.get_startup_action();
        let __shutdown_action = builder.get_shutdown_action();
        let mut __reaction = builder.add_reaction(name, Box::new(__trigger_inner));
        let mut __reaction = __reaction.with_action(
            __startup_action,
            0,
            ::boomerang::builder::TriggerMode::TriggersOnly,
        )?;
        Ok(__reaction)
    }
}

impl Trigger<Timeout> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut Duration) {
        ctx.schedule_shutdown(Some(*state))
    }
}
