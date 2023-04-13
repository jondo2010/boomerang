//! Test a particularly simple form of a distributed deterministic system where a federation that
//! receives timestamped messages has only those messages as triggers. Therefore, no additional
//! coordination of the advancement of time (HLA or Ptides) is needed.
//!
//! @author Edward A. Lee (original)

use boomerang::{
    builder::{BuilderReactionKey, EnvBuilder, Reactor, TypedPortKey},
    runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(state = "Source")]
struct SourceBuilder {
    #[reactor(output())]
    out: TypedPortKey<String>,

    #[reactor(reaction(function = "Source::reaction_startup"))]
    reaction_startup: BuilderReactionKey,
}

struct Source;
impl Source {
    #[boomerang::reaction(reactor = "SourceBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(effects)] out: &mut runtime::Port<String>,
    ) {
        println!("Sending 'Hello World!' message from source federate.");
        *out.get_mut() = Some("Hello World!".to_string());
        ctx.schedule_shutdown(None);
    }
}

#[derive(Reactor)]
#[reactor(state = "Destination")]
struct DestinationBuilder {
    #[reactor(input())]
    inp: TypedPortKey<String>,

    #[reactor(reaction(function = "Destination::reaction_in"))]
    reaction_in: BuilderReactionKey,

    #[reactor(reaction(function = "Destination::reaction_shutdown"))]
    reaction_shutdown: BuilderReactionKey,
}

struct Destination {
    received: bool,
}

impl Destination {
    #[boomerang::reaction(reactor = "DestinationBuilder")]
    fn reaction_in(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(triggers)] inp: &runtime::Port<String>,
    ) {
        let value = inp.get().as_ref().unwrap();
        println!(
            "At logical time {:?}, destination received: {value}",
            ctx.get_elapsed_logical_time()
        );
        if value != "Hello World!" {
            panic!("Expected to receive 'Hello World!'");
        }
        self.received = true;
    }

    #[boomerang::reaction(reactor = "DestinationBuilder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
        println!("Shutdown invoked.");
        assert!(self.received, "Destination did not receive the message.");
    }
}

#[derive(Reactor)]
#[reactor(
    state = "HelloDistributed",
    // This version preserves the timestamp.
    connection(from = "s.out", to = "d.inp")
)]
struct HelloDistributedBuilder {
    /// Reactor s is in federate Source
    #[reactor(child(state = "Source"))]
    s: SourceBuilder,

    /// Reactor d is in federate Destination
    #[reactor(child(state = "Destination {received: false}"))]
    d: DestinationBuilder,

    #[reactor(reaction(function = "HelloDistributed::reaction_startup"))]
    reaction_startup: BuilderReactionKey,
}

#[derive(Clone)]
struct HelloDistributed;

impl HelloDistributed {
    #[boomerang::reaction(reactor = "HelloDistributedBuilder", triggers(startup))]
    fn reaction_startup(&mut self, _ctx: &mut runtime::Context) {
        println!("Printing something in top-level federated reactor.");
    }
}

impl HelloDistributedBuilder {
    fn build_federated(
        name: &str,
        state: <Self as Reactor>::State,
        env: &mut EnvBuilder,
    ) -> Result<(), boomerang::builder::BuilderError>
    where
        <Self as Reactor>::State: Clone,
    {
        {
            let __s_state = Source;
            let mut __builder_s = env.add_reactor(&format!("_fed_{name}_s"), None, state.clone());
            let outputControlReactionTrigger =
                __builder_s.add_logical_action::<()>("__outputControlReactionTrigger", None)?;
            let s: SourceBuilder = __builder_s.add_child_reactor("s", __s_state)?;

            __builder_s
                .add_reaction(
                    "__reaction_s_out",
                    Box::new(
                        |_,
                         ctx,
                         inputs: &[runtime::IPort],
                         outputs: &mut [runtime::OPort],
                         actions: &mut [&mut runtime::Action]| {
                            // Sending from s.out in federate s to d.in in federate d
                        },
                    ),
                )
                .with_trigger_port(s.out, 0);
        }

        {
            let __d_state = Destination { received: false };
            let mut __builder_d = env.add_reactor(&format!("_fed_{name}_d"), None, state.clone());
            let d: DestinationBuilder = __builder_d.add_child_reactor("d", __d_state)?;
        }

        Ok(())
    }
}

#[test_log::test]
fn hello_distributed() {
    let mut env_builder = EnvBuilder::new();
    let _reactor = HelloDistributedBuilder::build(
        "hello_distributed",
        HelloDistributed,
        None,
        &mut env_builder,
    )
    .unwrap();
    let env = env_builder.try_into().expect("Error building environment!");
    let mut sched = runtime::Scheduler::new(env, true, false);
    sched.event_loop();
}
