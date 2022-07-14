use boomerang::{runtime, Reactor};
use std::convert::TryInto;

#[derive(Reactor)]
#[reactor(
    output(name = "y", type = "i32"),
    timer(name = "t"),
    reaction(function = "Source::reaction_t", triggers(timer="t"), effects("y"))
)]
struct Source {}
impl Source {
    fn reaction_t<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        _inputs: &SourceInputs,
        outputs: &SourceOutputs,
        _actions: &SourceActions,
    ) {
        sched.get_port_with_mut(outputs.y, |y, _is_set| {
            *y = 1;
            true
        });
    }
}

#[derive(Reactor)]
#[reactor(
    input(name = "x", type = "i32"),
    input(name = "y", type = "i32"),
    reaction(function = "Destination::reaction_x_y", triggers("x", "y"))
)]
struct Destination {}
impl Destination {
    fn reaction_x_y<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &DestinationInputs,
        _outputs: &DestinationOutputs,
        _actions: &DestinationActions,
    ) {
        let mut sum = 0;
        sched.get_port_with(inputs.x, |x: &i32, is_set| {
            if is_set {
                sum += *x;
            }
        });
        sched.get_port_with(inputs.y, |y: &i32, is_set| {
            if is_set {
                sum += *y;
            }
        });
        println!("Received {}", sum);
        assert!(sum == 2, "FAILURE: Expected 2.");
    }
}

#[derive(Reactor)]
#[reactor(
    input(name = "x", type = "i32"),
    output(name = "y", type = "i32"),
    reaction(function = "Pass::reaction_x", triggers("x"), effects("y"))
)]
struct Pass {}
impl Pass {
    fn reaction_x<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &PassInputs,
        outputs: &PassOutputs,
        _actions: &PassActions,
    ) {
        sched.get_port_with(inputs.x, |x: &i32, _is_set| {
            sched.get_port_with_mut(outputs.y, |y, _is_set| {
                *y = *x;
                true
            });
        });
    }
}

#[derive(Reactor)]
#[reactor(
    child(name = "s", reactor = "Source{}"),
    child(name = "d", reactor = "Destination{}"),
    child(name = "p1", reactor = "Pass{}"),
    child(name = "p2", reactor = "Pass{}"),
    connection(from = "s.y", to = "d.y"),
    connection(from = "s.y", to = "p1.x"),
    connection(from = "p1.y", to = "p2.x"),
    connection(from = "p2.y", to = "d.x"),
)]
struct Determinism {}
impl Determinism {}

#[test]
fn test() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    use boomerang::{builder::*, runtime};
    let mut env_builder = EnvBuilder::new();

    let (_, _, _) = Determinism {}
        .build("gain", &mut env_builder, None)
        .unwrap();

    //let gv = graphviz::build(&env_builder).unwrap();
    //let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    //std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    //let gv = graphviz::build_reaction_graph(&env_builder).unwrap();
    //let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
    //std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env);
    sched.start().unwrap();
}
