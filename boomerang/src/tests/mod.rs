use super::*;

use boomerang_derive::{reaction, Reactor};

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

#[derive(Eq, PartialEq, Reactor, Debug)]
struct HelloWorld<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    #[reactor(input)]
    x: u32,
    #[reactor(input)]
    y: u32,

    #[reactor(output)]
    o: u32,

    phantom: (PhantomData<V>, PhantomData<S>),
}

impl<V, S> HelloWorld<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    #[reaction((x, y) -> o)]
    fn foo(&mut self, scheduler: &mut S) {
        println!("Hello foo! {:?}", scheduler.get_elapsed_logical_time());
        self.x += 1;
        if self.x >= 5 {
            scheduler.stop();
        }
    }

    fn bar(&mut self, scheduler: &mut S) {
        println!("Hello bar! {:?}", scheduler.get_elapsed_logical_time());
        self.y += 1;
    }
}

fn schedule_destination<V, S>(dest: &Rc<RefCell<HelloWorld<V, S>>>, scheduler: &mut S)
where
    V: EventValue + 'static,
    S: Sched<V> + 'static,
{
    {
        let r1 = {
            let r1_dest = dest.clone();
            let r1_closure = Box::new(RefCell::new(move |sched: &mut S| {
                HelloWorld::foo(&mut (*r1_dest).borrow_mut(), sched);
            }));
            Rc::new(Reaction::new(r1_closure, 0, 0))
        };

        // timer foo(100 msec, 1000 msec)
        let foo_trigger = Rc::new(RefCell::new(Trigger {
            reactions: vec![r1],
            offset: Duration::from_millis(100),
            period: Some(Duration::from_millis(1000)),
            value: Rc::new(RefCell::new(None)),
            is_physical: false,
            scheduled: None,
            policy: QueuingPolicy::NONE,
        }));

        scheduler.schedule(foo_trigger, Duration::from_micros(0), None);
    }
}

#[test]
fn test2() {
    let mut sched = Scheduler::<&'static str>::new();

    let mut dest = Rc::new(RefCell::new(HelloWorld {
        x: 0,
        y: 0,
        o: 0,
        phantom: (PhantomData, PhantomData),
    }));

    schedule_destination::<&'static str, _>(&dest, &mut sched);

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
