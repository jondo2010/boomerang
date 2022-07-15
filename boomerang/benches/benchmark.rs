use criterion::{criterion_group, criterion_main, Criterion};

use boomerang::{
    builder::{BuilderActionKey, BuilderPortKey},
    runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(connection(from = "out1", to = "in1"))]
struct HelloBenchBuilder {
    #[reactor(timer(offset = "100 msec", period = "1 sec"))]
    tim1: BuilderActionKey,
    #[reactor(input())]
    in1: BuilderPortKey<u32>,
    #[reactor(output())]
    out1: BuilderPortKey<u32>,
    #[reactor(reaction(function = "HelloBench::foo"))]
    foo: runtime::ReactorKey,
    #[reactor(reaction(function = "HelloBench::bar"))]
    bar: runtime::ReactorKey,
}

pub struct HelloBench {
    my_i: u32,
}
impl HelloBench {
    #[boomerang::reaction(reactor = "HelloBenchBuilder", triggers("tim1"))]
    fn foo(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects)] out1: &mut runtime::Port<u32>,
    ) {
        //self.my_i += 1;
        //out1.set(self.my_i);
    }
    #[boomerang::reaction(reactor = "HelloBenchBuilder")]
    fn bar(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(triggers)] in1: &mut runtime::Port<u32>,
    ) {
        //if *in1.get() as usize >= 10000 {
        //    sched.stop();
        //}
    }
}

#[inline]
fn test() {}

pub fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("hello", |b| b.iter(|| test()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
