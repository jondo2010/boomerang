use boomerang::{Port, Sched, Scheduler};
use boomerang_derive::Reactor;

#[derive(Reactor, Debug, Default)]
#[reactor(
    timer(name="tim1", offset = "100 msec", period = "1 sec"),
    input(name="in1", type="u32"),
    output(name="out1", type="u32"),
    reaction(function="HelloWorld::foo", triggers("tim1"), uses(), effects("out1")),
    reaction(function="HelloWorld::bar", triggers("in1")),
    connection(from="out1", to="in1"),
    //child(reactor="Bar", name="my_bar", inputs("x.y", "y"), outputs("b")),
)]
pub struct HelloWorld {
    my_i: u32,
}

impl HelloWorld {
    fn foo<S: Sched>(&mut self, _sched: &mut S, _inputs: (), outputs: &mut Port<u32>) {
        let out1 = outputs;
        self.my_i += 1;
        out1.set(self.my_i);
        println!("foo, my_i={}", self.my_i);
    }
    fn bar<S: Sched>(&mut self, sched: &mut S, inputs: &mut Port<u32>, _outputs: ()) {
        let in1 = inputs;
        println!("bar, in1={}", in1.get());
        if *in1.get() == 5 {
            sched.stop();
        }
    }
}

#[test]
fn test() {
    type MySched = Scheduler<&'static str>;
    let mut sched = MySched::new();
    HelloWorld::create::<MySched>(&mut sched);
    while sched.next() && !sched.stop_requested {}
}
