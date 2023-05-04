//! Test a particularly simple form of a distributed deterministic system where a federation that
//! receives timestamped messages has only those messages as triggers. Therefore, no additional
//! coordination of the advancement of time (HLA or Ptides) is needed.
//!
//! @author Edward A. Lee (original)

use boomerang::{
    builder::{BuilderReactionKey, TypedPortKey},
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

#[derive(Clone)]
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
        //ctx.schedule_shutdown(None);
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

#[derive(Clone)]
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

#[cfg(feature = "federated")]
//#[test_log::test(tokio::test)]
#[tokio::test]
async fn hello_distributed() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .compact()
        .try_init()
        .unwrap();
    let _ = boomerang::runner::build_and_test_federation::<HelloDistributedBuilder>(
        "hello_distributed",
        HelloDistributed,
        false,
        true,
    )
    .await
    .unwrap();
}
