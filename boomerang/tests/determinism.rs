//#![feature(adt_const_params)]

use boomerang::{
    builder::{ActionPart, BuilderInputPort},
    runtime, Reactor,
};

#[derive(Reactor)]
struct SourceBuilder {
    #[reactor(output())]
    y: BuilderInputPort<i32>,
    #[reactor(timer())]
    t: ActionPart,
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
struct DestinationBuilder {
    #[reactor(input())]
    x: BuilderInputPort<i32>,
    #[reactor(input())]
    y: BuilderInputPort<i32>,
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
struct PassBuilder {
    #[reactor(input())]
    x: BuilderInputPort<i32>,
    #[reactor(output())]
    y: BuilderInputPort<i32>,
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

#[test]
fn test() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    use boomerang::{builder::*, runtime};
    let mut env_builder = EnvBuilder::new();

    let _ = DeterminismBuilder::build("determinism", (), None, &mut env_builder).unwrap();

    // let gv = graphviz::build(&env_builder).unwrap();
    // let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    // std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    // let gv = graphviz::build_reaction_graph(&env_builder).unwrap();
    // let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
    // std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    // env_builder.debug_info();

    let (env, dep_info) = env_builder.try_into().unwrap();

    runtime::check_consistency(&env, &dep_info);
    runtime::debug_info(&env, &dep_info);

    let sched = runtime::Scheduler::new(env, dep_info, true);
    sched.event_loop();
}
