use boomerang::{builder::*, runtime, Reactor};

#[derive(Reactor)]
struct TimeoutBuilder {
    #[reactor(reaction(function = "Timeout::reaction_startup"))]
    startup: runtime::ReactionKey,
}

struct Timeout {
    timeout: runtime::Duration,
}
impl Timeout {
    fn new(timeout: runtime::Duration) -> Self {
        Self { timeout }
    }

    #[boomerang::reaction(reactor = "TimeoutBuilder", triggers(startup))]
    fn reaction_startup(&mut self, ctx: &mut runtime::Context) {
        ctx.schedule_shutdown(Some(self.timeout))
    }
}

#[derive(Reactor)]
struct CountBuilder {
    #[reactor(timer(period = "1 sec"))]
    t: BuilderActionKey,
    #[reactor(output())]
    c: BuilderPortKey<u32>,
    #[reactor(child(state = "Timeout::new(runtime::Duration::from_secs(3))"))]
    timeout: TimeoutBuilder,
    #[reactor(reaction(function = "Count::reaction_t",))]
    reaction_t: runtime::ReactionKey,
}

struct Count(u32);
impl Count {
    #[boomerang::reaction(reactor = "CountBuilder", triggers(timer = "t"))]
    fn reaction_t(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects, path = "c")] xyc: &mut runtime::Port<u32>,
    ) {
        self.0 += 1;
        assert!(xyc.is_none());
        *xyc.get_mut() = Some(dbg!(self.0));
    }
}

#[test]
fn count() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();

    let _count = CountBuilder::build("count", Count(0), None, &mut env_builder).unwrap();

    let gv = graphviz::build(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let gv = graphviz::build_reaction_graph(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let (env, dep_info) = env_builder.try_into().unwrap();

    runtime::check_consistency(&env, &dep_info);
    runtime::debug_info(&env, &dep_info);

    let sched = runtime::Scheduler::new(env, dep_info, true);
    sched.event_loop();
}
