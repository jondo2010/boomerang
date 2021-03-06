/// Macro to generate the boilerplate for a test.
///
/// Run the test with the environment variable GRAPHS=any_value, and the test will also generate
/// GraphViz debug output into the current directory.
///
/// # Example
///
/// ```ignore
/// use boomerang::boomerang_test_body;
/// boomerang_test_body!(
///     /// Name of top-level Reactor.
///     my_reactor,
///     /// The ReactorBuilder type
///     MyReactorBuilder,
///     /// An expression for the initial state
///     MyReactor{}
/// );
/// ```
#[macro_export]
macro_rules! boomerang_test_body {
    ($name:ident, $reactor:ty, $state:expr) => {
        #[test]
        fn $name() {
            use boomerang::{builder::*, runtime};
            tracing_subscriber::fmt::init();
            let mut env_builder = EnvBuilder::new();
            let _ = <$reactor>::build(stringify!($name), $state, None, &mut env_builder)
                .expect("Error building top-level reactor!");

            if let Ok(_) = std::env::var("GRAPHS") {
                let gv = graphviz::create_full_graph(&env_builder).unwrap();
                let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
                std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

                let gv = graphviz::create_reaction_graph(&env_builder).unwrap();
                let mut f =
                    std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
                std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
            }

            let (env, dep_info) = env_builder.try_into().unwrap();

            runtime::util::assert_consistency(&env, &dep_info);
            runtime::util::print_debug_info(&env, &dep_info);

            let sched = runtime::Scheduler::new(env, dep_info, true);
            sched.event_loop();
        }
    };
}
