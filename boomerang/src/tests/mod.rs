use super::*;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use boomerang_derive::Reactor;

//#[derive(Reactor, Debug)]
struct HelloWorld<S>
where
    S: Sched,
    <S as Sched>::Value: EventValue,
{
    i: u32,

    //#[reactor(input)]
    input: Rc<RefCell<Port<u32>>>,

    //#[reactor(output)]
    output: Rc<RefCell<Port<u32>>>,

    phantom: PhantomData<S>,
}

impl<S> HelloWorld<S>
where
    S: Sched + 'static,
    <S as Sched>::Value: EventValue,
{
    //#[reaction((foo) -> output)]
    fn hello(&mut self, scheduler: &mut S) {
        println!("Hello foo! {:?}", scheduler.get_elapsed_logical_time());
        self.i += 1;
        if self.i >= 2 {
            scheduler.stop();
        }

        self.output.borrow_mut().set(self.i);
    }

    //#[reaction((input))]
    fn input(&mut self, scheduler: &mut S) {
        println!("Replying to input in={}", self.input.borrow().get());
    }

    // -----

    fn schedule(this: &Rc<RefCell<Self>>, scheduler: &mut S) {
        let reply_in_reaction = {
            let this_clone = this.clone();
            let closure = Box::new(RefCell::new(move |sched: &mut S| {
                Self::input(&mut (*this_clone).borrow_mut(), sched);
            }));
            Rc::new(Reaction::new(
                "reply_reaction",
                closure,
                u64::MAX,
                1,
                vec![],
            ))
        };

        let reply_in_trigger = Rc::new(Trigger {
            reactions: vec![reply_in_reaction],
            offset: None,
            period: None,
            value: Rc::new(RefCell::new(None)),
            is_physical: false,
            scheduled: RefCell::new(None),
            policy: QueuingPolicy::NONE,
        });

        let hello_reaction = {
            let this_clone = this.clone();
            let closure = move |sched: &mut S| {
                HelloWorld::hello(&mut (*this_clone).borrow_mut(), sched);
            };
            let closure = Box::new(RefCell::new(closure));
            Rc::new(Reaction::new(
                "hello_reaction",
                closure,
                0,
                0,
                vec![(this.borrow().output.clone(), vec![reply_in_trigger])],
            ))
        };

        // timer foo(100 msec, 1000 msec)
        let foo_trigger = Rc::new(Trigger {
            reactions: vec![hello_reaction],
            offset: Some(Duration::from_millis(100)),
            period: Some(Duration::from_millis(1000)),
            value: Rc::new(RefCell::new(None)),
            is_physical: false,
            scheduled: RefCell::new(None),
            policy: QueuingPolicy::NONE,
        });

        scheduler.schedule(foo_trigger, Duration::from_micros(0), None);
    }
}

impl<S> Reactor for HelloWorld<S>
where
    S: Sched,
    <S as Sched>::Value: EventValue,
{
    fn start_time_step(&self) {
        self.output.borrow_mut().reset();
    }
    fn start_timers<SS>(&self, sched: &mut SS) {
        unimplemented!()
    }
}

#[test]
fn test2() {
    let mut sched = Scheduler::<&'static str>::new();

    let output = Rc::new(RefCell::new(Port::new(0)));
    let input = output.clone();


    let mut dest = Rc::new(RefCell::new(HelloWorld {
        i: 0,
        output: output,
        input: input,
        phantom: PhantomData,
    }));

    HelloWorld::schedule(&dest, &mut sched);

    while sched.next() && !sched.stop_requested {}
    // sched.next();
}

// reactor Destination {
// input x:int;
// input y:int;
// reaction(x, y) {=
// printf("Time since start: %lld.\n", get_elapsed_logical_time());
// if (x_is_present) {
// printf("  x is present.\n");
// }
// if (y_is_present) {
// printf("  y is present.\n");
// }
// =}
// }
