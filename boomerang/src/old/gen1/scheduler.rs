use std::collections::BTreeSet;
use std::rc::Rc;

use tracing::{event, instrument};

use super::{Duration, Event, EventValue, Instant, QueuingPolicy, Reaction, Trigger};

const INITIAL_REACT_QUEUE_SIZE: usize = 10;
const INITIAL_EVENT_QUEUE_SIZE: usize = 10;

/// Top-level Reactor trait
pub trait Reactor: std::fmt::Debug {
    type Sched: Sched;

    /// Invoke code that must execute before starting a new logical time round, such as initializing
    /// outputs to be absent.
    fn start_time_step(&self);

    /// Get the startup free-running timers
    fn get_starting_timers(&self) -> Box<[Rc<Trigger<Self::Sched>>]>;

    /// Function to wrap up execution. If this returns true, then one more invocation of next() will
    /// be executed in order to invoke reactions that are triggered by shutdown.
    fn wrapup(&self) -> bool;
}

/// Scheduler trait
pub trait Sched: Sized + std::fmt::Debug {
    /// EventValue type
    type Value;
    // type Timer;
    // type Input<T>;
    // type Output<T>;

    /// Return the elpased logical time in nanoseconds since the start of execution.
    fn get_elapsed_logical_time(&self) -> Duration;

    /// Return the current logical time in nanoseconds since January 1, 1970.
    fn get_logical_time(&self) -> Instant;

    /// Return the current physical time in nanoseconds since January 1, 1970.
    fn get_physical_time(&self) -> Instant;

    /// Return the elapsed physical time.
    // fn get_elapsed_physical_time(&self) -> Duration;

    /// Print a snapshot of the priority queues used during execution.
    fn print_snapshot(&self) {}

    /// Function to request stopping execution at the end of the current logical time.
    fn stop(&mut self);

    /// Schedule the specified trigger at current_time plus the offset of the specified trigger plus
    /// the delay. The value is required to be a pointer returned by malloc because it will be freed
    /// after having been delivered to all relevant destinations unless it is NULL, in which case it
    /// will be ignored. If the trigger offset plus the extra delay is greater than zero and stop
    /// has been requested, then ignore this and return 0. Also, if the trigger argument is null,
    /// ignore and return 0. Otherwise, return a handle to the scheduled trigger, which is an
    /// integer greater than 0.
    fn schedule(
        &mut self,
        trigger: &Rc<Trigger<Self>>,
        extra_delay: Duration,
        value: Option<Self::Value>,
    );
}

#[derive(Debug)]
pub struct Scheduler<V>
where
    V: EventValue,
{
    /// Reference to the main reactor
    main_reactor: Box<dyn Reactor<Sched = Self>>,

    /// Indicator of whether to wait for physical time to match logical time. By default, execution
    /// will wait. The command-line argument -fast will eliminate the wait and allow logical time
    /// to exceed physical time.
    fast: bool,
    /// Current time in nanoseconds since January 1, 1970. This is not in scope for reactors.
    current_time: Instant,

    /// Logical time at the start of execution.
    start_time: Instant,

    /// Physical time at the start of the execution.
    physical_start_time: std::time::SystemTime,

    /// Indicator that the execution should stop after the completion of the current logical time.
    /// This can be set to true by calling the `stop()` function in a reaction.
    stop_requested: bool,

    /// The logical time to elapse during execution, or -1 if no timeout time has been given. When
    /// the logical equal to start_time + duration has been reached, execution will terminate.
    duration: Option<Instant>,

    /// Stop time (start_time + duration), or 0 if no timeout time has been given.
    stop_time: Option<Instant>,

    /// Indicator of whether the keepalive command-line option was given.
    keepalive_specified: bool,

    // Priority queues.
    /// For sorting by time.
    event_q: BTreeSet<Box<Event<Self>>>,
    /// For sorting by deadline
    reaction_q: BTreeSet<Rc<Reaction<Self>>>,
    /// For recycling malloc'd events.
    recycle_q: Vec<Box<Event<Self>>>,
}

impl<V> Scheduler<V>
where
    V: EventValue,
{
    pub fn new(main_reactor: Box<dyn Reactor<Sched = Self>>, fast: bool) -> Self {
        Self {
            main_reactor,
            fast,
            current_time: Instant::now(),
            start_time: Instant::now(),
            physical_start_time: std::time::SystemTime::now(),
            stop_requested: false,
            duration: None,
            stop_time: None,
            keepalive_specified: false,

            // event_q: BTreeSet::with_capacity(INITIAL_REACT_QUEUE_SIZE),
            // reaction_q: BTreeSet::with_capacity(INITIAL_REACT_QUEUE_SIZE),
            event_q: BTreeSet::new(),
            reaction_q: BTreeSet::new(),
            recycle_q: Vec::with_capacity(INITIAL_EVENT_QUEUE_SIZE),
        }
    }

    #[instrument]
    pub fn execute(&mut self) {
        // initialize();
        // event!(tracing::Level::INFO, ?self.current_time, ?local_deadline, "Deadline violation.");

        for trigger in self.main_reactor.get_starting_timers().into_iter() {
            self.schedule(&trigger, Duration::from_micros(0), None);
        }

        while self.next() && !self.stop_requested {}

        self.wrapup();
        self.termination();
    }

    pub fn print_event_queue(&self) {
        use tabular::{row, Table};
        let mut table = Table::new("  {:>}  {:<}  {:<}");
        table.add_heading("Event Queue:");
        table.add_row(row!("Time", "Trigger", "Value"));
        for ev in self.event_q.iter() {
            table.add_row(row!(
                format!("{:?}", ev.time - self.start_time),
                format!("{:p}", ev.trigger),
                format!("{:?}", ev.value),
            ));
        }
        println!("{}", table);
    }

    pub fn print_reaction_queue(&self) {
        use tabular::{row, Table};
        let mut table = Table::new("  {:<}  {:<}  {:<}");
        table.add_heading("Reaction Queue:");
        table.add_row(row!("chain_id", "index", "reaction"));
        for reac in self.reaction_q.iter() {
            table.add_row(row!(
                format!("{:?}", reac.chain_id),
                format!("{:?}", reac.index),
                // format!("{:#?}", reac.reactor),
                format!(".."),
            ));
        }
        println!("{}", table);
    }

    /// Search for first Event with matching trigger, bounded by upper time limit.
    /// LinguaFranca calls this "pqueue_find_equal()"
    /// Find the highest-ranking item with priority up to and including the given maximum
    /// priority that matches the supplied entry.
    fn find_event_bounded(
        &self,
        event: &Box<Event<Self>>,
        limit: &Instant,
    ) -> Option<&Box<Event<Self>>> {
        let rng = self.event_q.range::<Box<Event<Self>>, _>(event..);
        let found = rng.rev().find(|&x| {
            println!("ev: {:?}", x.time);
            (x.trigger.as_ref() as *const Trigger<Self>
                == event.trigger.as_ref() as *const Trigger<Self>)
                || x.time == *limit
        });
        if let Some(f2) = found {
            println!("found: {:?}", f2.time);
        }
        found.into()
    }

    /// For the specified reaction, if it has produced outputs, insert the resulting triggered
    /// reactions into the reaction queue.
    #[instrument]
    fn schedule_output_reactions(&mut self, reaction: &Rc<Reaction<Self>>) {
        // If the reaction produced outputs, put the resulting triggered reactions into the blocking
        // queue.
        for out_reaction in reaction
            .triggers
            .iter()
            .filter_map(|(present, out_triggers)| {
                if present.borrow().is_present() {
                    Some(out_triggers)
                } else {
                    None
                }
            })
            .flatten()
            .flat_map(|trigger| trigger.reactions.iter().cloned().collect::<Vec<Rc<_>>>())
        {
            self.reaction_q.insert(out_reaction);
        }
    }

    /// Advance logical time to the lesser of `logical_time` or the `stop_time`, if set.
    ///
    /// If the `fast` option is not enabled, then wait until physical time matches or exceeds the
    /// start time of execution plus the current_time plus the specified logical time. If this is
    /// not interrupted, then advance `current_time` by the specified logical_delay.
    ///
    /// Return `true` if time advanced to the time of the event and `false` if the wait was
    /// interrupted or if the timeout time was reached.
    fn wait_until(&mut self, logical_time: Instant) -> bool {
        let (logical_time, stop_time_reached) = self
            .stop_time
            .filter(|stop_time| logical_time > *stop_time)
            .map_or((logical_time, false), |stop_time| {
                // Indicate on return that the time of the event was not reached.
                // We still wait for time to elapse in case asynchronous events come in.
                (stop_time, true)
            });

        let return_value = if !self.fast {
            event!(
                tracing::Level::INFO,
                ?logical_time,
                "Waiting for logical time"
            );

            logical_time
                .checked_duration_since(Instant::now())
                .map_or(false, |time_to_wait| {
                    std::thread::sleep(time_to_wait);
                    stop_time_reached
                })
        } else {
            stop_time_reached
        };
        // Advance current time.
        self.current_time = logical_time;
        return_value
    }

    /// Wait until physical time matches or exceeds the time of the least tag on the event queue. If
    /// there is no event in the queue, return 0. After this wait, advance current_time to match
    /// this tag. Then pop the next event(s) from the event queue that all have the same tag, and
    /// extract from those events the reactions that are to be invoked at this logical time. Sort
    /// those reactions by index (determined by a topological sort) and then execute the reactions
    /// in order. Each reaction may produce outputs, which places additional reactions into the
    /// index-ordered priority queue. All of those will also be executed in order of indices. If the
    /// -timeout option has been given on the command line, then return 0 when the logical time
    /// duration matches the specified duration. Also return 0 if there are no more events in the
    /// queue and the keepalive command-line option has not been given. Otherwise, return 1.
    #[instrument]
    pub fn next(&mut self) -> bool {
        let event = self.event_q.last();
        if event.is_none() && !self.keepalive_specified {
            // No event in the queue.
            return true;
        }

        // If there is no next event and -keepalive has been specified on the command line, then we
        // will wait the maximum time possible.
        let mut next_time = event
            .map(|ev| ev.time)
            .unwrap_or(Instant::now() + Duration::from_secs(1000u64));

        let event_trigger = event.unwrap().trigger.clone();

        // Wait until physical time >= event.time.
        // The wait_until function will advance current_time.
        if !self.wait_until(next_time) {
            // Sleep was interrupted or the timeout time has been reached.
            // Time has not advanced to the time of the event.
            // There may be a new earlier event on the queue.
            let new_event = self.event_q.last();
            if new_event
                .filter(|ev| {
                    ev.trigger.as_ref() as *const Trigger<Self>
                        == event_trigger.as_ref() as *const Trigger<Self>
                })
                .is_some()
            {
                // There is no new event. If the timeout time has been reached, or if the maximum
                // time has been reached (unlikely), then return.
                if self
                    .stop_time
                    .map(|stop_time| self.current_time >= stop_time)
                    .unwrap_or(false)
                    || new_event.is_none()
                {
                    self.stop_requested = true;
                    return true;
                }
            } else {
                // Handle the new event.
                // FIXME: this actually does nothing.
                let event = new_event;
                next_time = event.expect("Unexpected None() event.").time;
            }
        }

        // Invoke code that must execute before starting a new logical time round, such as
        // initializing outputs to be absent.
        self.main_reactor.start_time_step();

        // Pop all events from event_q with timestamp equal to current_time, extract all the
        // reactions triggered by these events, and stick them into the reaction queue.
        loop {
            let event = self.event_q.pop_last().expect("Should be some");
            event!(tracing::Level::INFO, %event, "Handling event");

            // Load reactions triggered by this event onto the reaction queue.
            for reaction in event.trigger.reactions.iter() {
                event!(
                    tracing::Level::INFO,
                    "Pushed on reaction_q reaction \"{}\"",
                    reaction.name
                );
                self.reaction_q.insert(reaction.clone());
            }

            if !event.trigger.is_physical && event.trigger.period.is_some() {
                // Reschedule the trigger.
                // NOTE: the delay here may be negative because the schedule function will add
                // the trigger.offset, which we don't want at this point.
                self.schedule(
                    &event.trigger,
                    event.trigger.period.unwrap_or(Duration::from_micros(0))
                        - event.trigger.offset.unwrap_or_default(),
                    None,
                );
            }

            // Copy the value pointer into the trigger struct so that the reactions can access
            // it.
            *event.trigger.value.borrow_mut() = *event.value.borrow();

            // Recycle the event
            self.recycle_q.push(event);

            // Peek at the next event in the event queue.
            // If the event time differs from current_time, or there is no event, break out of the
            // loop.
            if self
                .event_q
                .last()
                .map(|ev| ev.time != self.current_time)
                .unwrap_or(true)
            {
                break;
            }
        }

        // Invoke reactions.
        while let Some(reaction) = self.reaction_q.pop_last() {
            event!(
                tracing::Level::INFO,
                "Popped from reaction_q \"{}\" with deadline: {:?}",
                reaction.name,
                reaction.local_deadline.as_ref().map(|x| x.0)
            );

            // If the reaction has a deadline, compare to current physical time and invoke the
            // deadline violation reaction instead of the reaction function if a violation has
            // occurred.
            // NOTE: the violation reaction will be invoked at most once per logical time value. If
            // the violation reaction triggers the same reaction at the current time value, even if
            // at a future superdense time, then the reaction will be invoked and the violation
            // reaction will not be invoked again.
            let violation = if let Some((local_deadline, ref local_deadline_callback)) =
                reaction.local_deadline
            {
                // Get the current physical time.
                // struct timespec current_physical_time;
                // clock_gettime(CLOCK_REALTIME, &current_physical_time);
                // Convert to instant_t.
                // instant_t physical_time = current_physical_time.tv_sec * BILLION +
                // current_physical_time.tv_nsec;
                let physical_time = Instant::now();
                // Check for deadline violation.
                // There are currently two distinct deadline mechanisms:
                // 1. Local deadlines are defined with the reaction;
                // 2. Container deadlines are defined in the container.
                // They can have different deadlines, so we have to check both.
                // Handle the local deadline first.
                if physical_time > self.current_time + local_deadline {
                    event!(tracing::Level::INFO, ?self.current_time, ?local_deadline, "Deadline violation.");
                    // Deadline violation has occurred. Invoke the local handler, if there is one.
                    if (&mut *local_deadline_callback.borrow_mut())(self) {
                        // If the reaction produced outputs, put the resulting triggered reactions
                        // into the queue.
                        self.schedule_output_reactions(&reaction);
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !violation {
                // Invoke the reaction function.
                (&mut *reaction.reactor.borrow_mut())(self);

                // If the reaction produced outputs, put the resulting triggered reactions into the
                // queue.
                self.schedule_output_reactions(&reaction);
            }
        }

        // No more reactions should be blocked at this point.
        // assert(pqueue_size(blocked_q) == 0);

        if self
            .stop_time
            .map(|stop_time| self.current_time >= stop_time)
            .unwrap_or(false)
        {
            self.stop_requested = true;
            return false;
        }

        return true;
    }

    /// Invoke any code generated wrapup. If this returns true, then actions have been scheduled at
    /// the next microstep. Invoke next() one more time to react to those actions.
    fn wrapup(&mut self) {
        if self.main_reactor.wrapup() {
            self.next();
        }
    }

    /// Print a final report of elapsed times.
    fn termination(&self) {
        let elapsed_logical_time = self.current_time - self.start_time;
        println!(
            "---- Elapsed logical time (in nsec): {:?}",
            elapsed_logical_time
        );
        println!(
            "---- Elapsed physical time (in nsec): {:?}",
            self.start_time.elapsed()
        );
    }
}

impl<V> Sched for Scheduler<V>
where
    V: EventValue,
{
    type Value = V;
    // type Timer = Rc<RefCell<Trigger<Self>>>;
    // type Input<T> = Rc<RefCell<Port<T>>>;
    // type Output<T> = Rc<RefCell<Port<T>>>;

    fn get_elapsed_logical_time(&self) -> Duration {
        self.current_time - self.start_time
    }

    fn get_logical_time(&self) -> Instant {
        self.current_time
    }

    fn get_physical_time(&self) -> Instant {
        Instant::now()
    }

    // fn get_elapsed_physical_time() -> Duration {
    // self
    // }

    fn stop(&mut self) {
        self.stop_requested = true;
    }

    #[instrument]
    fn schedule(
        &mut self,
        trigger: &Rc<Trigger<Self>>,
        extra_delay: Duration,
        value: Option<Self::Value>,
    ) {
        // Compute the tag.  How we do that depends on whether this is a logical or physical action.
        let mut tag = self.current_time;
        // event_t* existing = NULL;

        // Recycle event_t structs, if possible.
        let mut e = self.recycle_q.pop().map_or_else(
            || Box::new(Event::new(tag, trigger.clone(), value)),
            |mut e| {
                e.trigger = trigger.clone();
                *e.value.borrow_mut() = value;
                e
            },
        );

        // For logical actions, the logical time of the new event is just the current logical time
        // plus the minimum offset (action parameter) plus the extra delay specified in the call to
        // schedule.
        e.time = tag + trigger.offset.unwrap_or_default() + extra_delay;

        if trigger.is_physical {
            // If the trigger is physical, then we need to use physical time and the time of the
            // last invocation to adjust the tag. Specifically, the timestamp assigned to the action
            // event will be the maximum of the current logical time, the current physical time, and
            // the time of last invocation plus the minTime (action parameter) plus the extra_delay
            // (argument to this function). If the action has never been scheduled before, then the
            // timestamp will be the maximum of the current logical time, the current physical time,
            // and the start time + minTime + extra_delay.
            // Get the current physical time.
            let physical_time = Instant::now();
            if physical_time > self.current_time {
                tag = physical_time;
            }

            let min_inter_arrival = trigger.offset.unwrap_or_default() + extra_delay;

            // Compute the earliest time that this event can be scheduled.
            let earliest_time = trigger
                .scheduled
                .borrow()
                .map_or(self.start_time + min_inter_arrival, |scheduled| {
                    scheduled + min_inter_arrival
                });

            if earliest_time > tag {
                // The event is early. See which policy applies.
                match trigger.policy {
                    QueuingPolicy::UPDATE => {
                        // Update existing event if it exists.
                        e.time = tag;
                        // See if there is an existing event up to but not including
                        // the earliest time this event can be scheduled.
                        if let Some(existing) =
                            self.find_event_bounded(&e, &(earliest_time - Duration::from_nanos(1)))
                        {
                            // Update the value of the existing event.
                            *existing.value.borrow_mut() = value;
                        }
                    }
                    QueuingPolicy::DROP => {
                        // Recycle the new event.
                        *e.value.borrow_mut() = None;
                        self.recycle_q.push(e);
                        return;
                    }
                    QueuingPolicy::DEFER => {
                        // if (trigger->policy == DEFER || (trigger->policy == UPDATE && existing ==
                        // NULL)) Adjust the tag.
                        tag = earliest_time;
                    }
                    QueuingPolicy::NONE => {}
                }
            }

            // Record the tag.
            *trigger.scheduled.borrow_mut() = Some(tag);
            e.time = tag;
        }
        // Do not schedule events if a stop has been requested.
        if tag != self.current_time && self.stop_requested {
            return;
        }

        // Handle duplicate events for logical actions.
        if !trigger.is_physical {
            // existing = pqueue_find_equal_same_priority(event_q, e);
            // if (existing != NULL) {
            // existing->value = value;
            // Recycle the new event.
            // e->value = NULL;    // FIXME: Memory leak.
            // pqueue_insert(recycle_q, e);
            // return(0);
            // }
        }

        event!(tracing::Level::INFO, %e, "Inserting Event");
        // println!("Inserting Event {:#?}", &e);

        // NOTE: There is no need for an explicit microstep because when this is called, all events
        // at the current tag (time and microstep) have been pulled from the queue, and any new
        // events added at this tag will go into the reaction_q rather than the event_q, so anything
        // put in the event_q with this same time will automatically be executed at the next
        // microstep.
        self.event_q.insert(e);

        // FIXME: make a record of handle and implement unschedule.
        // NOTE: Rather than wrapping around to get a negative number, we reset the handle on the
        // assumption that much earlier handles are irrelevant. int return_value =
        // __handle++; if (__handle < 0) __handle = 1;
        // return return_value;
        // }
    }
}
