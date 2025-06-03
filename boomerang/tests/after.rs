//! This checks that the after keyword adjusts logical time, not using physical time.

use boomerang::prelude::*;

#[reactor]
fn Foo(#[input] x: i32, #[output] y: i32) -> impl Reactor2 {
    builder
        .add_reaction2(None)
        .with_trigger(x)
        .with_effect(y)
        .with_reaction_fn(|_ctx, _state, (x, mut y)| {
            *y = x.map(|x| 2 * x);
        })
        .finish()?;
}

#[reactor]
fn Print(
    #[state(default = Duration::milliseconds(10))] expected_time: Duration,
    #[state] i: usize,
    #[input] x: i32,
) -> impl Reactor2 {
    builder
        .add_reaction2(None)
        .with_trigger(x)
        .with_reaction_fn(|ctx, state, (x,)| {
            state.i += 1;
            let elapsed_time = ctx.get_elapsed_logical_time();
            println!("Result is {:?}", *x);
            assert_eq!(*x, Some(84), "Expected result to be 84");
            println!("Current logical time is: {}", elapsed_time);
            println!("Current physical time is: {:?}", ctx.get_physical_time());
            assert_eq!(
                elapsed_time, state.expected_time,
                "Expected logical time to be {}",
                state.expected_time
            );
            state.expected_time += Duration::seconds(1);
        })
        .finish()?;

    builder
        .add_reaction2(None)
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, _| {
            println!("Final result is {}", state.i);
            assert!(state.i != 0, "ERROR: Final reactor received no data.");
        })
        .finish()?;
}

#[reactor]
fn After() -> impl Reactor2 {
    let f = builder.add_child_reactor2(Foo(), "foo", Default::default(), false)?;
    let p = builder.add_child_reactor2(Print(), "print", Default::default(), false)?;
    let t = builder.add_timer("t", TimerSpec::default().with_period(Duration::SECOND))?;
    builder.connect_port(f.y, p.x, Some(Duration::milliseconds(10)), false)?;

    builder
        .add_reaction2(None)
        .with_trigger(t)
        .with_effect(f.x)
        .with_reaction_fn(|ctx, _state, (_t, mut x)| {
            *x = Some(42);
            let elapsed_time = ctx.get_elapsed_logical_time();
            println!("Timer @ {elapsed_time}!");
        })
        .finish()?;
}

#[test]
fn main() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(3));
    let _ = boomerang_util::runner::build_and_test_reactor2(
        After(),
        "after",
        Default::default(),
        config,
    )
    .unwrap();
}

mod foo {
    use boomerang::prelude::*;
    use boomerang_builder::{BuilderReactorKey, ReactorBuilderState};

    pub trait Reactor3: Sized {
        type Ports;
        type State: runtime::ReactorData;

        fn build(
            &self,
            name: &str,
            state: Self::State,
            parent: Option<BuilderReactorKey>,
            bank_info: Option<runtime::BankInfo>,
            is_enclave: bool,
            env: &mut EnvBuilder,
        ) -> Result<Self::Ports, BuilderError>;
    }

    pub trait ReactorPorts3 {
        /// The fields of the Ports struct (e.g. the ports)
        type Fields;
        /// Build the reactor with the given closure
        fn build_with<F, S>(f: F) -> impl Reactor3<State = S, Ports = Self>
        where
            F: Fn(&mut ReactorBuilderState<'_, S>, Self::Fields) -> Result<(), BuilderError>
                + 'static,
            S: runtime::ReactorData;
    }

    impl<F, State, Ports> Reactor3 for F
    where
        F: Fn(
                /*name*/ &str,
                /*state*/ State,
                /*parent*/ Option<BuilderReactorKey>,
                /*bank_info*/ Option<boomerang_runtime::BankInfo>,
                /*is_enclave*/ bool,
                /*env*/ &mut EnvBuilder,
            ) -> Result<Ports, BuilderError>
            + 'static,
        State: runtime::ReactorData,
    {
        type Ports = Ports;
        type State = State;
        fn build(
            &self,
            name: &str,
            state: State,
            parent: Option<BuilderReactorKey>,
            bank_info: Option<boomerang_runtime::BankInfo>,
            is_enclave: bool,
            env: &mut EnvBuilder,
        ) -> Result<Self::Ports, BuilderError> {
            (self)(name, state, parent, bank_info, is_enclave, env)
        }
    }

    struct FooPorts {
        pub x: ::boomerang::builder::TypedPortKey<
            i32,
            ::boomerang::builder::Input,
            ::boomerang::builder::Contained,
        >,
        pub y: ::boomerang::builder::TypedPortKey<
            i32,
            ::boomerang::builder::Output,
            ::boomerang::builder::Contained,
        >,
    }
    impl ReactorPorts3 for FooPorts {
        type Fields = (
            ::boomerang::builder::TypedPortKey<
                i32,
                ::boomerang::builder::Input,
                ::boomerang::builder::Local,
            >,
            ::boomerang::builder::TypedPortKey<
                i32,
                ::boomerang::builder::Output,
                ::boomerang::builder::Local,
            >,
        );
        fn build_with<F, S>(f: F) -> impl Reactor3<State = S, Ports = FooPorts>
        where
            F: Fn(
                    &mut ::boomerang::builder::ReactorBuilderState<'_, S>,
                    Self::Fields,
                ) -> Result<(), ::boomerang::builder::BuilderError>
                + 'static,
            S: ::boomerang::runtime::ReactorData,
        {
            move |name: &str,
                  state: S,
                  parent: Option<::boomerang::builder::BuilderReactorKey>,
                  bank_info: Option<::boomerang::runtime::BankInfo>,
                  is_enclave: bool,
                  env: &mut ::boomerang::builder::EnvBuilder| {
                let mut builder = env.add_reactor(name, parent, bank_info, state, is_enclave);
                let x = builder.add_port::<i32, ::boomerang::builder::Input>("x", None)?;
                let y = builder.add_port::<i32, ::boomerang::builder::Output>("y", None)?;
                f(&mut builder, (x, y))?;
                builder.finish()?;
                Ok(FooPorts {
                    x: x.contained(),
                    y: y.contained(),
                })
            }
        }
    }
    type FooState = ();
    #[allow(non_snake_case)]
    fn Foo() -> impl Reactor3<State = FooState, Ports = FooPorts> {
        <FooPorts as ReactorPorts3>::build_with::<_, FooState>(move |builder, (x, y)| {
            {
                builder
                    .add_reaction2(None)
                    .with_trigger(x)
                    .with_effect(y)
                    .with_reaction_fn(|_ctx, _state, (x, mut y)| {
                        *y = x.map(|x| 2 * x);
                    })
                    .finish()?;
            }
            Ok(())
        })
    }
}

mod foobar {
    trait Foo {
        type T;
        fn bar(&self, t: Self::T);
    }

    impl<F, A> Foo for F
    where
        F: Fn(A),
    {
        type T = A;
        fn bar(&self, t: Self::T) {
            (self)(t);
        }
    }
}
