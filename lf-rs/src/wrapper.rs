extern "C" {
    fn __test();
    fn next();
}

// #[no_mangle]
// extern "C" fn __initialize_trigger_objects() {
// println!("I'm called from C");
// }

#[test]
pub fn test1() {
    unsafe {
        use crate::bindings::*;
        initialize();
        __start_timers();
        print_snapshot();
        next();
        print_snapshot();


        /*
        let helloworldtest_a_reaction_0 = reaction_t {
            function: &helloworld_rfunc_0,
            self_: &mut helloworldtest_a_self as *mut ::std::os::raw::c_void,
            index: 0,
            chain_id: 0,
            pos: 0,
            num_outputs: 1,
            output_produced: helloworldtest_a_reaction_0_outputs_are_present as *mut *mut bool_,
            triggered_sizes: &helloworldtest_a_reaction_0_triggered_sizes[0]
                as *mut ::std::os::raw::c_int,
            triggers: &helloworldtest_a_reaction_0_triggers[0] as *mut *mut *mut trigger_t,
            running: bool__false_,
            local_deadline: 0,
            deadline_violation_handler: NULL,
        };

        let helloworldtest_a_foo_trigger_reactions =
            [&mut helloworldtest_a_reaction_0 as *mut reaction_t];

        let helloworldtest_a_foo_trigger = trigger_t {
            reactions: helloworldtest_a_foo_trigger_reactions.as_mut_ptr(),
            number_of_reactions: 1,
            offset: 0,
            period: 0,
            value: std::ptr::null() as *mut std::ffi::c_void,
            is_physical: bool__false_,
            scheduled: NEVER,
            policy: queuing_policy_t_NONE,
        };

        __schedule(
            (&mut helloworldtest_a_foo_trigger) as *mut trigger_t,
            0,
            std::ptr::null() as *mut std::ffi::c_void,
        );
        */
    }
}
