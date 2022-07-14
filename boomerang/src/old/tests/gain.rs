#![feature(option_expect_none)]

// Ported example from the Wiki.
// Scale(scale:int(2)) {
//      input x1:int;
//      input x2:int;
//      input x3:int;
//      output y:int;
//      reaction(x1) -> y {=
//          set(y, x1 * self->scale);
//      =}
//      reaction(x2) {= =}
//      reaction(x3) {= =}
//      y -> x2;
// }
// reactor Test {
//     input x:int;
//     state received_value:bool(false);
//     reaction(x) {=
//         printf("Received %d.\n", x);
//         self->received_value = true;
//         if (x != 2) { printf("ERROR: Expected 2!\n"); exit(1); }
//     =}
//     reaction(shutdown) {=
//         if (!self->received_value) { printf("ERROR: No value received by Test reactor!\n"); }
//         else { printf("Test passes.\n"); }
//     =}
// }
// main reactor Gain {
//     g = new Scale();
//     d = new Test();
//     g.y -> d.x;
//     g.y -> g.x3
//     reaction(startup) -> g.x {=
//         set(g.x, 1);
//     =}
// }
use boomerang::*;

// pub struct Input<'a, T, const X: &'static str>(&'a InPort<T>);
// external_inputs: (Input<u32, "x">,),

/// Create Scale
/// inputs:
///     external_inputs:    (in_x1, in_x3)
///     external_outputs:   ((out_y, [out_y sens triggers], callback for out_y sens triggers), )
///
///     F: ([reacs sens to in_x1], [reacs sens to in_x3]) -> [trigs sens to out_y]
/// returns:
///     ([reacs sens to in_x1], [reacs sens to in_x3],)
fn scale_create<S, F>(
    input_x1: Option<&InPort<u32>>,
    input_x2: Option<&InPort<u32>>,
    input_x3: Option<&InPort<u32>>,
    output_y: (&OutPort<u32>, Box<[&Rc<Trigger<S>>]>, F),
) -> (Box<[Rc<Reaction<S>>]>, Box<[Rc<Reaction<S>>]>)
where
    S: Sched,
    S::Value: std::fmt::Debug,
    F: FnOnce(&[Rc<Reaction<S>>], &[Rc<Reaction<S>>]) -> Box<[Rc<Trigger<S>>]>,
{
    // unpack
    let (out_y, out_y_triggers, out_y_triggers_callback) = output_y;

    let in_x1 = input_x1
        .cloned()
        .unwrap_or(Rc::new(RefCell::new(Port::<u32>::new(0))));
    input_x2.expect_none("x2 is internally connected and not available.");
    let in_x2 = out_y.clone();
    let in_x3 = input_x3
        .cloned()
        .unwrap_or(Rc::new(RefCell::new(Port::<u32>::new(0))));

    let reaction_x3 = {
        let in_x3_clone = in_x3.clone();
        Rc::new(Reaction::new(
            "scale_reaction_x3",
            Box::new(RefCell::new(move |s: &mut S| {
                println!("x3={}", in_x3_clone.borrow().get());
            })),
            0,
            0,
            vec![], // this reaction has no outputs
        ))
    };

    let out_y_cb_triggers = out_y_triggers_callback(&[], &[reaction_x3.clone()]);

    let reaction_x2 = {
        let in_x2_clone = in_x2.clone();
        Rc::new(Reaction::new(
            "scale_reaction_x2",
            Box::new(RefCell::new(move |s: &mut S| {
                println!("x2={}", in_x2_clone.borrow().get());
            })),
            0,
            0,
            vec![], // this reaction has no outputs
        ))
    };
    let trig_in_x2 = Rc::new(Trigger::new(
        vec![reaction_x2],
        None,
        None,
        false,
        QueuingPolicy::NONE,
    ));

    // external+internal triggers on out_y
    let output_triggers_y: OutputTrigger<S> = (
        out_y.clone(),
        out_y_triggers
            .into_iter()
            .cloned()
            .chain(out_y_cb_triggers.iter())
            .chain([trig_in_x2].iter())
            .cloned()
            .collect(),
    );

    // reaction(x) -> y
    let reaction_x = {
        // collect requisites
        let in_x_clone = in_x1.clone();
        let out_y_clone = out_y.clone();
        Rc::new(Reaction::new(
            "scale_reaction_x",
            Box::new(RefCell::new(move |s: &mut S| {
                println!("scale_reaction_x x1={}", in_x_clone.borrow().get());
                out_y_clone.borrow_mut().set(in_x_clone.borrow().get() * 2);
            })),
            0,
            0,
            vec![output_triggers_y],
        ))
    };

    (
        // reactions sensitive to in_x1
        Box::new([reaction_x.clone()]),
        // reactions sensitive to in_x3
        Box::new([reaction_x3.clone()]),
    )
}

fn test_create<S: Sched>(
    input_x: Option<&InPort<u32>>,
    external_outputs: (),
) -> (Box<[Rc<Reaction<S>>]>,) {
    let in_x = input_x
        .cloned()
        .unwrap_or(Rc::new(RefCell::new(Port::<u32>::new(0))));
    let reaction_x = Rc::new(Reaction::new(
        "test_reaction_x",
        Box::new(RefCell::new(move |s: &mut S| {
            println!("test_reaction_x x={}", in_x.borrow().get());
        })),
        0,
        0,
        vec![], // this reaction has no outputs
    ));
    (
        // reactions sensitive to in_x
        Box::new([reaction_x.clone()]),
    )
}

fn gain_create<S>() -> Rc<Trigger<S>>
where
    S: Sched,
    S::Value: std::fmt::Debug,
{
    let out_g_y = Rc::new(RefCell::new(Port::<u32>::new(0)));
    let in_d_x = out_g_y.clone();
    let (in_d_x_sensitive_reactions,) = test_create::<S>(Some(&in_d_x), ());
    let trig_in_d_x = Rc::new(Trigger::new(
        in_d_x_sensitive_reactions.to_vec(),
        None,
        None,
        false,
        QueuingPolicy::NONE,
    ));
    let output_trigger_g_y: (_, Box<[_]>, _) = (
        &out_g_y,
        Box::new([&trig_in_d_x]),
        |x1: &[Rc<Reaction<S>>], x3: &[Rc<Reaction<S>>]| -> Box<[Rc<Trigger<S>>]> {
            // connect y -> x3
            let trig_in_g_x3 = Rc::new(Trigger::new(
                x3.to_vec(),
                None,
                None,
                false,
                QueuingPolicy::NONE,
            ));
            Box::new([trig_in_g_x3])
        },
    );
    let in_g_x = Rc::new(RefCell::new(Port::<u32>::new(0)));
    let (in_g_x1_sensitive_reactions, in_g_x2_sensitive_reactions) =
        scale_create::<S, _>(Some(&in_g_x.clone()), None, None, output_trigger_g_y);
    let trig_in_g_x = Rc::new(Trigger::new(
        in_g_x1_sensitive_reactions.to_vec(),
        None,
        None,
        false,
        QueuingPolicy::NONE,
    ));
    let output_trigger_startup: OutputTrigger<S> = (in_g_x.clone(), vec![trig_in_g_x]);
    let reaction_startup = Rc::new(Reaction::new(
        "gain_reaction_startup",
        Box::new(RefCell::new(move |s: &mut S| {
            // set(g.x, 1);
            println!("gain_reaction_startup g.x=1");
            in_g_x.borrow_mut().set(1);
        })),
        0,
        0,
        [output_trigger_startup].to_vec(),
    ));
    let trig_startup = Rc::new(Trigger::new(
        vec![reaction_startup.clone()],
        None,
        None,
        false,
        QueuingPolicy::NONE,
    ));
    trig_startup
}

#[derive(Debug)]
struct Gain<S>
where
    S: Sched,
    S::Value: std::fmt::Debug,
{
    trig_startup: Rc<Trigger<S>>,
}

impl<S> Reactor for Gain<S>
where
    S: Sched,
    S::Value: std::fmt::Debug,
{
    type Sched = S;
    fn start_time_step(&self) {}
    fn get_starting_timers(&self) -> Box<[Rc<Trigger<Self::Sched>>]> {
        Box::new([self.trig_startup.clone()])
    }
    fn wrapup(&self) -> std::primitive::bool {
        false
    }
}

#[test]
fn test() {
    // tracing_subscriber::fmt().compact().init();

    let react = Box::new(Gain {
        trig_startup: gain_create::<Scheduler<()>>(),
    });

    // let react = HelloWorldTest::create_reactor();
    let mut sched = Scheduler::<()>::new(react, false);
    sched.execute();
}
