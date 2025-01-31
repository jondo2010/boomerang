use boomerang::builder::{TimerActionKey, TypedPortKey};
use boomerang::prelude::*;
use boomerang::runtime;
use boomerang_builder::ReactorBuilderState;
use boomerang_derive::reactor2;

type Input<T> = TypedPortKey<T, boomerang::builder::Input>;
type Output<T> = TypedPortKey<T, boomerang::builder::Output>;
type Timer = TimerActionKey;


#[derive(::boomerang::typed_builder_macro::TypedBuilder)]
#[builder(
    crate_module_path = ::boomerang::typed_builder,
    mutators(
        fn build_ports(&mut self, builder: &mut ReactorBuilderState) {
            self.x = <Input<u32> as boomerang::builder::ReactorField>::build("x", (), builder).unwrap();
            self.y = <Output<u32> as boomerang::builder::ReactorField>::build("y", (), builder).unwrap();
        }
    )
)]
#[allow(non_snake_case)]
pub struct ScaleProps {
    #[builder(setter(doc = "**scale**: [`u32`]"), default = 2)]
    pub scale: u32,
    #[builder(via_mutators)]
    pub x: Input<u32>,
    #[builder(via_mutators)]
    pub y: Output<u32>,
}

trait ScaleTypes {
    type X;
    type Y;
}

impl ScaleTypes for Scale {
    type X = u32;
    type Y = u32;
}

pub trait ReactorProps {
    type Builder: ::boomerang::typed_builder::TypedBuilder;
    fn builder(
            name: &str,
            env: &mut boomerang::builder::EnvBuilder,
            parent: Option<::boomerang::builder::BuilderReactorKey>,
            bank_info: Option<::boomerang::runtime::BankInfo>,
            is_enclave: bool,
    ) -> Self::Builder;
}

impl ReactorProps for ScaleProps {
    type Builder = ScalePropsBuilder;
    fn builder(
            name: &str,
            env: &mut boomerang::builder::EnvBuilder,
            parent: Option<::boomerang::builder::BuilderReactorKey>,
            bank_info: Option<::boomerang::runtime::BankInfo>,
            is_enclave: bool,
    ) -> Self::Builder {
        ScaleProps::builder()

    let mut reactor_builder = env.add_reactor(name, parent, bank_info, (), is_enclave);
    let props = props_builder.build_ports(&mut reactor_builder).build();
    }
}

//#[reactor2]
fn Scale(//#[prop(default = 2)]
    //scale: u32,
    //#[input]
    //x: u32,
    //#[output]
    //y: u32,
    props_builder: ScalePropsBuilder
) -> ScaleProps {
    todo!()
}

#[test]
fn gain2() {

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
