use boomerang::{IsPresent, Port, Sched, Scheduler};
use boomerang_derive::Reactor;

#[derive(Reactor, Debug, Default)]
#[reactor(
    timer(name = "foo", offset = "100 msec", period = "1000 msec"),
    output(name = "c", type = "i32"),
    input(name = "inp", type = "i32"),
    reaction(function = "HelloWorld::foo", triggers("foo"), effects("c")),
    reaction(function = "HelloWorld::in1", triggers("inp")),
    reaction(function = "HelloWorld::in2", triggers("inp"))
)]
struct HelloWorld {
    i: i32,
}

impl HelloWorld {
    fn foo<S: Sched>(&mut self, _sched: &mut S, _inputs: (), outputs: &mut Port<i32>) {
        let c = outputs;
        println!("Hello World.");
        self.i += 1;
        c.set(self.i);
    }

    fn in1<S: Sched>(&mut self, _sched: &mut S, inputs: &mut Port<i32>, _outputs: ()) {
        let inp = inputs;
        println!("Replying1 to input in = {}.", inp.get());
    }

    fn in2<S: Sched>(&mut self, _sched: &mut S, inputs: &mut Port<i32>, _outputs: ()) {
        let inp = inputs;
        println!("Replying1 to input in = {}.", inp.get());
    }
}

#[derive(Reactor, Debug, Default)]
#[reactor(
    //child(reactor="HelloWorld", name="hello", inputs("inp"), outputs("c")),
    //connection(from="hello.c", to="hello.inp"),
)]
struct HelloWorldTest {}

#[test]
fn test() {
    // tracing_subscriber::fmt::init();
    let react = HelloWorldTest::create_reactor();
    let mut sched = Scheduler::<()>::new(react, false);
    sched.execute();
}
