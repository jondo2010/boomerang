use boomerang::builder::{TimerActionKey, TypedPortKey};
use boomerang::prelude::*;
use boomerang::runtime;
use boomerang_builder::ReactorBuilderState;
use boomerang_derive::reactor2;

type Input<T> = TypedPortKey<T, boomerang::builder::Input>;
type Output<T> = TypedPortKey<T, boomerang::builder::Output>;
type Timer = TimerActionKey;

#[reactor2]
fn Scale(#[prop(default = 2)] scale: u32, #[input] x: u32, #[output] y: u32) {}

#[test]
fn gain2() {
    #[automatically_derived]
    struct ScaleProps {
        scale: u32,
        pub x: Input<u32>,
        pub y: Output<u32>,
    }

    #[automatically_derived]
    impl ScaleProps {
        fn build(
            builder: &mut ReactorBuilderState,
        ) -> Result<Self, boomerang::builder::BuilderError> {
            let x = <Input<u32> as boomerang::builder::ReactorField>::build("x", (), builder)?;
            let y = <Output<u32> as boomerang::builder::ReactorField>::build("y", (), builder)?;
            Ok(Self { x, y })
        }
    }

    #[automatically_derived]
    trait ScaleTypes {
        type X;
        type Y;
    }

    #[automatically_derived]
    impl ScaleTypes for Scale {
        type X = u32;
        type Y = u32;
    }

    impl Scale {
        fn assemble(
            name: &str,
            env: &mut boomerang::builder::EnvBuilder,
            parent: Option<::boomerang::builder::BuilderReactorKey>,
            bank_info: Option<::boomerang::runtime::BankInfo>,
            is_enclave: bool,
        ) -> Result<Self, boomerang::builder::BuilderError> {
            type X = <Scale as ScaleTypes>::X;
            type Y = <Scale as ScaleTypes>::Y;

            let mut __builder = env.add_reactor(name, parent, bank_info, (), is_enclave);
            let scale = Scale::build(&mut __builder)?;

            /* reaction(x) -> y {} */
            let __reaction = __builder
                .add_reaction("", |_| {
                    runtime::reaction_closure!(ctx, _reactor, _ref_ports, _mut_ports, _actions => {
                        let x: runtime::InputRef<X> = _ref_ports.partition().unwrap();
                        let mut y: runtime::OutputRef<Y> = _mut_ports.partition_mut().unwrap();
                        *y = Some(2 * x.unwrap());
                    })
                    .into()
                })
                .with_port(scale.x, 0, boomerang_builder::TriggerMode::TriggersAndUses)?
                .with_port(scale.y, 0, boomerang_builder::TriggerMode::EffectsOnly)?
                .finish()?;

            let __reactor = __builder.finish()?;

            Ok(scale)
        }
    }

    //#[reactor]
    fn Test(
        //#[input] x: u32,
        props: TestProps,
    ) {
    }

    struct TestProps {
        pub x: Input<u32>,
    }

    struct GainProps {}

    fn Gain(props: GainProps) {
        //g: Scale,
        //t: Test,
        //tim: Timer,
    }

    impl Gain {
        fn assemble() {
            /*

            reaction(tim) -> g.x {
            }
            */
        }
    }
}
