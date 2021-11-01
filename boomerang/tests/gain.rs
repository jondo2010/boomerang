// Example in the Wiki.
use boomerang::{runtime, Reactor};
use std::convert::TryInto;

#[derive(Reactor)]
#[reactor(
    input(name = "x", type = "u32"),
    output(name = "y", type = "u32"),
    reaction(function = "Scale::reaction_x", triggers("x"), effects("y"))
)]
struct Scale {
    scale: u32,
}
impl Scale {
    fn reaction_x<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &ScaleInputs,
        outputs: &ScaleOutputs,
        _actions: &ScaleActions,
    ) {
        sched.get_port_with(inputs.x, |x: &u32, _is_set| {
            sched.get_port_with_mut(outputs.y, |y, _is_set| {
                *y = *x * self.scale;
                true
            });
        });
    }
}

#[derive(Reactor)]
#[reactor(
    input(name = "x", type = "i32"),
    reaction(function = "Test::reaction_x", triggers("x"))
)]
struct Test {}
impl Test {
    fn reaction_x<S: runtime::SchedulerPoint>(
        &mut self,
        sched: &S,
        inputs: &TestInputs,
        _outputs: &TestOutputs,
        _actions: &TestActions,
    ) {
        sched.get_port_with(inputs.x, |x: &i32, _is_set| {
            println!("Received {}", x);
            assert!(*x == 2, "Expected 2!");
        })
    }
}

#[derive(Reactor)]
#[reactor(
    timer(name = "tim"),
    reaction(function = "Gain::reaction_tim", triggers(timer="tim"),
        //effects("g.x")
    ),
    child(name = "g", reactor = "Scale{scale: 2}"),
    child(name = "t", reactor = "Test{}"),
    connection(from = "g.y", to = "t.x")
)]
struct Gain {}
impl Gain {
    fn reaction_tim<S: runtime::SchedulerPoint>(
        &mut self,
        _sched: &S,
        _inputs: &GainInputs,
        _outputs: &GainOutputs,
        _actions: &GainActions,
    ) {
        // g.x.set(1);
    }
}

#[test]
fn test() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    use boomerang::{builder::*, runtime};
    let mut env_builder = EnvBuilder::new();

    let (_, _, _) = Gain {}.build("gain", &mut env_builder, None).unwrap();

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    let mut sched = runtime::Scheduler::new(env);
    sched.start().unwrap();
}
