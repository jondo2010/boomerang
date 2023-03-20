use boomerang::{
    builder::{BuilderActionKey, TypedPortKey},
    run, runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(state = "Source")]
struct SourceBuilder {
    #[reactor(output())]
    y: TypedPortKey<i32>,
    #[reactor(timer())]
    t: BuilderActionKey,
    #[reactor(reaction(function = "Source::reaction_t",))]
    reaction_t: runtime::ReactionKey,
}

struct Source;
impl Source {
    #[boomerang::reaction(reactor = "SourceBuilder", triggers(timer = "t"))]
    fn reaction_t(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(effects)] y: &mut runtime::Port<i32>,
    ) {
        *y.get_mut() = Some(1);
    }
}

#[derive(Reactor)]
#[reactor(state = "Destination")]
struct DestinationBuilder {
    #[reactor(input())]
    x: TypedPortKey<i32>,
    #[reactor(input())]
    y: TypedPortKey<i32>,
    #[reactor(reaction(function = "Destination::reaction_x_y"))]
    reaction_x_y: runtime::ReactionKey,
}

struct Destination;
impl Destination {
    #[boomerang::reaction(reactor = "DestinationBuilder")]
    fn reaction_x_y(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(triggers)] x: &runtime::Port<i32>,
        #[reactor::port(triggers)] y: &runtime::Port<i32>,
    ) {
        let mut sum = 0;
        if let Some(x) = *x.get() {
            sum += x;
        }
        if let Some(y) = *y.get() {
            sum += y;
        }
        println!("Received {}", sum);
        assert_eq!(sum, 2, "FAILURE: Expected 2.");
    }
}

#[derive(Reactor)]
#[reactor(state = "Pass")]
struct PassBuilder {
    #[reactor(input())]
    x: TypedPortKey<i32>,
    #[reactor(output())]
    y: TypedPortKey<i32>,
    #[reactor(reaction(function = "Pass::reaction_x"))]
    reaction_x: runtime::ReactionKey,
}

struct Pass;
impl Pass {
    #[boomerang::reaction(reactor = "PassBuilder")]
    fn reaction_x(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(triggers)] x: &runtime::Port<i32>,
        #[reactor::port(effects)] y: &mut runtime::Port<i32>,
    ) {
        *y.get_mut() = *x.get();
    }
}

#[derive(Reactor)]
#[reactor(
    connection(from = "s.y", to = "d.y"),
    connection(from = "s.y", to = "p1.x"),
    connection(from = "p1.y", to = "p2.x"),
    connection(from = "p2.y", to = "d.x")
)]
#[allow(dead_code)]
struct DeterminismBuilder {
    #[reactor(child(state = "Source"))]
    s: SourceBuilder,
    #[reactor(child(state = "Destination"))]
    d: DestinationBuilder,
    #[reactor(child(state = "Pass"))]
    p1: PassBuilder,
    #[reactor(child(state = "Pass"))]
    p2: PassBuilder,
}

fn main() {
    tracing_subscriber::fmt::init();
    let _ = run::build_and_run_reactor::<DeterminismBuilder>("determinism", ()).unwrap();
}
