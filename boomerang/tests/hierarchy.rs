use std::sync::{Arc, RwLock};

use boomerang::{
    builder::ReactorTypeBuilderState,
    runtime::{self, PortIndex},
};
use tracing::event;

// Test data transport across hierarchy.
// target Cpp;
// reactor Source {
//     output out:int;
//     timer t;
//     reaction(t) -> out {=
//         out.set(1);
//     =}
// }
// reactor Gain {
//     input in:int;
//     output out:int;
//     reaction(in) -> out {=
//         out.set((*in.get()) * 2);
//     =}
// }
// reactor Print {
//     input in:int;
//     reaction(in) {=
//         auto value = *in.get();
//         std::cout << "Received: " << value << std::endl;
//         if (value != 2) {
//             std::cerr << "Expected 2." << std::endl;
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

struct Print {
    inp: PortIndex,
}

impl Print {
    fn build(builder: &mut ReactorTypeBuilderState) -> Arc<RwLock<Self>> {
        let inp = builder.add_input::<u32>("in").unwrap();

        let this = Arc::new(RwLock::new(Self { inp }));
        let this2 = this.clone();

        builder
            .add_reaction("r0", move |sched| {
                this2.write().unwrap().r0(sched);
            })
            .with_trigger_port(inp)
            .finish();

        this
    }

    fn r0(&mut self, sched: &runtime::SchedulerPoint) {
        let value = sched.get_port::<u32>(self.inp);
        event!(tracing::Level::INFO, "Received {:?}", value);
        assert!(matches!(value, Some(2)));
    }
}

#[test]
fn hierarchy() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    use boomerang::builder::*;
    use boomerang::runtime;

    let mut env_builder = EnvBuilder::new();

    let source = env_builder.add_reactor_type("Source", |mut builder| {
        let out = builder.add_output::<u32>("out").unwrap();
        let t = builder.add_startup_timer("t");
        let _ = builder
            .add_reaction("r0", move |sched| {
                sched.set_port(out, 1u32);
                event!(
                    tracing::Level::INFO,
                    "Sent {:?}",
                    sched.get_port::<u32>(out)
                );
            })
            .with_trigger_action(t)
            .with_antidependency(out)
            .finish();
        builder.finish()
    });

    let gain = env_builder.add_reactor_type("Gain", |mut builder| {
        let inp = builder.add_input::<u32>("in").unwrap();
        let out = builder.add_output::<u32>("out").unwrap();
        let _ = builder
            .add_reaction("r0", move |sched| {
                let in_val: u32 = sched.get_port(inp).unwrap();
                sched.set_port(out, in_val * 2);
            })
            .with_trigger_port(inp)
            .with_antidependency(out)
            .finish();
        builder.finish()
    });

    // let gain_container = env_builder.add_reactor_type(
    // "GainContainer",
    // move |mut builder: ReactorTypeBuilderState| {
    // let inp = builder.add_input::<u32>("in").unwrap();
    // let out1 = builder.add_output::<u32>("out1").unwrap();
    // let out2 = builder.add_output::<u32>("out2").unwrap();
    // let gain0 = builder.add_child_instance("gain0", gain);
    //
    // builder.finish()
    // },
    // );

    let print = env_builder.add_reactor_type("Print", |mut builder| {
        let _print = Print::build(&mut builder);
        builder.finish()
    });

    let hierarchy = env_builder.add_reactor_type("Hierarchy", move |mut builder| {
        let source0 = builder.add_child_instance("source0", source);
        let gain0 = builder.add_child_instance("gain0", gain);
        // let container = builder.add_child_instance("container0", gain_container);
        let print0 = builder.add_child_instance("print0", print);
        let print1 = builder.add_child_instance("print1", print);
        builder.add_connection(source0, "out", gain0, "in");
        builder.add_connection(gain0, "out", print0, "in");
        builder.add_connection(gain0, "out", print1, "in");
        builder.finish()
    });

    let env = env_builder.build(hierarchy);
    // println!("{:#?}", &env);
    let environment = env.build().unwrap();

    let mut sched = runtime::Scheduler::new(environment.max_level());
    sched.start(environment).unwrap();
}
