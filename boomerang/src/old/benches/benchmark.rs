use criterion::{criterion_group, criterion_main, Criterion};

use boomerang::{Port, Sched, Scheduler};
use boomerang_derive::Reactor;

#[derive(Reactor, Debug, Default)]
#[reactor(
    timer(name = "tim1", offset = "100 msec", period = "1 sec"),
    input(name = "in1", type = "u32"),
    output(name = "out1", type = "u32"),
    reaction(function = "HelloBench::foo", triggers("tim1"), effects("out1")),
    reaction(function = "HelloBench::bar", triggers("in1")),
    connection(from = "out1", to = "in1")
)]
pub struct HelloBench {
    my_i: u32,
}
impl HelloBench {
    fn foo<S: Sched>(&mut self, _sched: &mut S, _inputs: (), out1: &mut Port<u32>) {
        self.my_i += 1;
        out1.set(self.my_i);
    }
    fn bar<S: Sched>(&mut self, sched: &mut S, in1: &mut Port<u32>, _outputs: ()) {
        if *in1.get() as usize >= 10000 {
            sched.stop();
        }
    }
}

#[inline]
fn test() {
    let reactor = HelloBench::create_reactor();
    let mut sched = Scheduler::<()>::new(reactor, true);
    sched.execute();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("hello", |b| b.iter(|| test()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
