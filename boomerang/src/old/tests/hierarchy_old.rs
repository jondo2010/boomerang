// Test data transport across hierarchy.
// reactor Source {
//     output out:int;
//     timer t;
//     reaction(t) -> out {=
//         set(out, 1);
//     =}
// }
// reactor Gain {
//     input in:int;
//     output out:int;
//     reaction(in) -> out {=
//         printf("Gain received %d.\n", in);
//         set(out, in * 2);
//     =}
// }
// reactor Print {
//     input in:int;
//     reaction(in) {=
//         printf("Received: %d.\n", in);
//         if (in != 2) {
//             printf("Expected 2.\n");
//             exit(1);
//         }
//     =}
// }
// reactor GainContainer {
//     input in:int;
//     output out:int;
//     output out2:int;
//     gain = new Gain();
//     in -> gain.in;
//     gain.out -> out;
//     gain.out -> out2;
// }
// main reactor Hierarchy {
//     source = new Source();
//     container = new GainContainer();
//     print = new Print();
//     print2 = new Print();
//     source.out -> container.in;
//     container.out -> print.in;
//     container.out -> print2.in;
// }

use boomerang::*;
// use boomerang_derive::Reactor;

// #[derive(Reactor, Debug, Default)]
// #[reactor(
// output(name = "out", type = "i32"),
// timer(name = "t"),
// reaction(function = "Source::r0", triggers("t"), effects("out"))
// )]
// pub struct Source {}
// impl Source {
// fn r0(&self) -> () {}
// }
//
// #[derive(Reactor, Debug, Default)]
// #[reactor(
// input(name = "in", type = "i32"),
// output(name = "out", type = "i32"),
// reaction(function = "Gain::r0", triggers("in"), uses(), effects("out"))
// )]
// pub struct Gain {}
//
// impl Gain {
// fn reaction(&self, in) -> out {=
// printf("Gain received %d.\n", in);
// set(out, in * 2);
// =}
// }
//
// #[derive(Reactor, Debug, Default)]
// #[reactor(
// input(name = "in1", type = "i32"),
// output(name = "out1", type = "i32"),
// output(name = "out2", type = "i32"),
// child(class = "Gain", name = "gain", inputs("in1"), outputs("out")),
// connection(from = "in1", to = "gain.in1"),
// connection(from = "gain.out", to = "out1"),
// connection(from = "gain.out", to = "out2")
// )]
// pub struct GainContainer {}

#[derive(Reactor, Debug, Default)]
#[reactor(
    child(class="Source", name="s", outputs("out")),
    child(class="GainContainer", name="c", inputs("in"), outputs("out1", "out2")),
    child(class="Print", name="p1", inputs("in"))
    child(class="Print", name="p2", inputs("in"))
    connection(from="s.out", to="c.in"),
    connection(from="c.out1", to="p1.in"),
    connection(from="c.out2", to="p2.in"),
)]
pub struct Hierarchy {}

//#[cfg(not(test))]
mod hand_written {
    use super::*;
    use builder::{ReactionBuilder, ReactorBuildable, ReactorBuilder, TimerBuilder};

    struct Source {}
    struct SourceInputs {}

    impl std::default::Default for SourceInputs {
        fn default() -> Self {
            Self {}
        }
    }
    struct SourceOutputs {
        __out: Rc<RefCell<Port<u32>>>,
    }
    impl std::default::Default for SourceOutputs {
        fn default() -> Self {
            Self {
                __out: Rc::new(RefCell::new(Port::new(std::default::Default::default()))),
            }
        }
    }
    impl ReactorBuildable for Source {
        type Inputs = SourceInputs;
        type Outputs = SourceOutputs;
        fn create<S: Sched>(inputs: Self::Inputs) -> (ReactorBuilder<S>, Self::Outputs) {
            // First deal with fixed portion
            let __out = Rc::new(RefCell::new(Port::<u32>::new(0)));
            let __react0 = {
                let __out = __out.clone();
                Box::new(RefCell::new(move |s: &mut S| {
                    println!("Source R0");
                    __out.borrow_mut().set(1);
                }))
            };
            let __outputs = Self::Outputs { __out };

            // Now ReactorBuilder
            let __reactor = {
                let __t = Rc::new(TimerBuilder {
                    offset: None,
                    period: None,
                });
                let reaction0 = ReactionBuilder {
                    reaction: __react0,
                    depends_on_timers: vec![__t.clone()],
                    depends_on_inputs: vec![],
                    provides_outputs: vec![],
                };
                ReactorBuilder {
                    timers: [("t".to_owned(), __t)].iter().cloned().collect(),
                    inputs: [].iter().cloned().collect(),
                    outputs: [].iter().cloned().collect(),
                    children: [].iter().cloned().collect(),
                    reactions: vec![reaction0],
                }
            };

            (__reactor, __outputs)
        }
    }

    struct Gain {}
    struct GainInputs {
        __in: Rc<RefCell<Port<u32>>>,
    }
    impl std::default::Default for GainInputs {
        fn default() -> Self {
            Self {
                __in: Rc::new(RefCell::new(Port::<u32>::default())),
            }
        }
    }
    struct GainOutputs {
        __out: Rc<RefCell<Port<u32>>>,
    }
    impl std::default::Default for GainOutputs {
        fn default() -> Self {
            Self {
                __out: Rc::new(RefCell::new(Port::<u32>::default())),
            }
        }
    }
    impl ReactorBuildable for Gain {
        type Inputs = GainInputs;
        type Outputs = GainOutputs;
        fn create<S: Sched>(inputs: Self::Inputs) -> (ReactorBuilder<S>, Self::Outputs) {
            // First deal with fixed portion
            let GainInputs { __in } = inputs;
            let __out = Rc::new(RefCell::new(Port::<u32>::new(0)));
            let __react0 = {
                let __in = __in.clone();
                let __out = __out.clone();
                Box::new(RefCell::new(move |s: &mut S| {
                    println!("Gain R0 in={}", __in.borrow().get());
                    __out.borrow_mut().set(__out.borrow().get() * 2);
                }))
            };
            let __outputs = GainOutputs { __out };

            // Now ReactorBuilder
            let __reactor = {
                let reaction0 = ReactionBuilder {
                    reaction: __react0,
                    depends_on_timers: vec![],
                    depends_on_inputs: vec![],
                    provides_outputs: vec![],
                };
                ReactorBuilder {
                    timers: [].iter().cloned().collect(),
                    inputs: [].iter().cloned().collect(),
                    outputs: [].iter().cloned().collect(),
                    children: [].iter().cloned().collect(),
                    reactions: vec![reaction0],
                }
            };

            (__reactor, __outputs)
        }
    }

    // mod builtin {
    // use super::*;
    // struct GenDelay<T> {}
    // struct GenDelayInputs<T> {
    // __in: Rc<RefCell<Port<T>>>,
    // }
    // impl<T: Default> std::default::Default for GenDelayInputs<T> {
    // fn default() -> Self {
    // Self {
    // __in: Rc::new(RefCell::new(Port::<T>::default())),
    // }
    // }
    // }
    // struct GenDelayOutputs<T> {
    // __out: Rc<RefCell<Port<T>>>,
    // }
    // impl<T: Default> std::default::Default for GenDelayOutputs<T> {
    // fn default() -> Self {
    // Self {
    // __out: Rc::new(RefCell::new(Port::<T>::default())),
    // }
    // }
    // }
    // impl<T: Default> ReactorBuildable for GenDelay<T> {
    // type Inputs = GenDelayInputs<T>;
    // type Outputs = GenDelayOutputs<T>;
    // fn create<S: Sched>(inputs: Self::Inputs) -> (ReactorBuilder<S>, Self::Outputs) {
    // let Self::Inputs { __in, .. } = inputs;
    //
    // let __r0_react = {
    // let __in = __in.clone();
    // let __out = __out.clone();
    // Box::new(RefCell::new(move |s: &mut S| {
    // println!("Gain R0 in={}", __in.borrow().get());
    // __out.borrow_mut().set(__out.borrow().get() * 2);
    // }))
    // };
    // }
    // }
    // }

    struct Container {}
    struct ContainerInputs {
        __in: Rc<RefCell<Port<u32>>>,
    }
    impl std::default::Default for ContainerInputs {
        fn default() -> Self {
            Self {
                __in: Rc::new(RefCell::new(Port::<u32>::default())),
            }
        }
    }
    struct ContainerOutputs {
        __out: Rc<RefCell<Port<u32>>>,
        __out2: Rc<RefCell<Port<u32>>>,
    }
    impl std::default::Default for ContainerOutputs {
        fn default() -> Self {
            Self {
                __out: Rc::new(RefCell::new(Port::<u32>::default())),
                __out2: Rc::new(RefCell::new(Port::<u32>::default())),
            }
        }
    }

    impl ReactorBuildable for Container {
        type Inputs = ContainerInputs;
        type Outputs = ContainerOutputs;

        fn create<S: Sched>(inputs: Self::Inputs) -> (ReactorBuilder<S>, Self::Outputs) {
            // First deal with fixed portion
            let Self::Inputs { __in, .. } = inputs;

            // self.in -> gain.in
            let __g_in = __in.clone();

            // Gain g
            let (__g_reactor, __g_out) = {
                type Inputs = <Gain as ReactorBuildable>::Inputs;
                type Outputs = <Gain as ReactorBuildable>::Outputs;
                let (__g_reactor, Outputs { __out: __g_out, .. }) =
                    <Gain as ReactorBuildable>::create::<S>(Inputs {
                        __in: __g_in,
                        ..std::default::Default::default()
                    });
                (__g_reactor, __g_out)
            };

            // gain.out -> self.out
            let __out = __g_out.clone();

            // gain.out -> self.out2
            let __out2 = __g_out.clone();

            let __outputs = Self::Outputs {
                __out,
                __out2,
                ..std::default::Default::default()
            };

            // Now ReactorBuilder
            let __reactor_builder = {
                ReactorBuilder {
                    timers: [].iter().cloned().collect(),
                    inputs: [].iter().cloned().collect(),
                    outputs: [].iter().cloned().collect(),
                    children: [("g".to_owned(), Rc::new(__g_reactor))]
                        .iter()
                        .cloned()
                        .collect(),
                    reactions: vec![],
                }
            };

            (__reactor_builder, __outputs)
        }
    }

    pub struct Hierarchy {}

    impl ReactorBuildable for Hierarchy {
        type Inputs = ();
        type Outputs = ();

        fn create<S: Sched>(inputs: Self::Inputs) -> (ReactorBuilder<S>, Self::Outputs) {
            // let Self::Inputs { .. } = inputs;

            // Source s
            let (__s_reactor, __s_out) = {
                type Inputs = <Source as ReactorBuildable>::Inputs;
                type Outputs = <Source as ReactorBuildable>::Outputs;
                let (__s_reactor, Outputs { __out: __s_out, .. }) =
                    <Source as ReactorBuildable>::create::<S>(Inputs {
                        ..std::default::Default::default()
                    });
                (__s_reactor, __s_out)
            };

            // source.out -> container.in
            let __c_in = __s_out.clone();

            // Container c
            let (__c_reactor, __c_out) = {
                type Inputs = <Container as ReactorBuildable>::Inputs;
                type Outputs = <Container as ReactorBuildable>::Outputs;
                let (__c_reactor, Outputs { __out: __c_out, .. }) =
                    <Container as ReactorBuildable>::create::<S>(Inputs {
                        __in: __c_in,
                        ..std::default::Default::default()
                    });
                (__c_reactor, __s_out)
            };

            // Print p1
            // Print p2
            let __outputs = ();

            let __reactor = {
                ReactorBuilder {
                    timers: [].iter().cloned().collect(),
                    inputs: [].iter().cloned().collect(),
                    outputs: [].iter().cloned().collect(),
                    children: [
                        ("s".to_owned(), Rc::new(__s_reactor)),
                        ("c".to_owned(), Rc::new(__c_reactor)),
                    ]
                    .iter()
                    .cloned()
                    .collect(),
                    reactions: vec![],
                }
            };

            (__reactor, __outputs)
        }
    }
}

#[test]
fn test() {
    // let reactor_builder = hand_written::Hierarchy::create_main::<Scheduler<()>>();
}
