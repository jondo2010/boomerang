use boomerang::builder::{TimerActionKey, TypedPortKey};
use boomerang::prelude::*;
use boomerang::runtime;
use boomerang_builder::{ReactorBuilderState, TimerSpec};
use boomerang_derive::reactor2;
use boomerang_runtime::{refs, BoxedReactionFn};

type Input<T> = TypedPortKey<T, boomerang::builder::Input>;
type Output<T> = TypedPortKey<T, boomerang::builder::Output>;
type Timer = TimerActionKey;

#[derive(::boomerang::typed_builder_macro::TypedBuilder)]
#[builder(crate_module_path = ::boomerang::typed_builder)]
#[allow(non_snake_case)]
pub struct ScaleProps {
    #[builder(setter(doc = "**scale**: [`u32`]"), default = 2)]
    pub scale: u32,
    #[builder(setter(doc = "**x**: [`u32`]"))]
    pub x: Input<u32>,
    #[builder(setter(doc = "**y**: [`u32`]"))]
    pub y: Output<u32>,
}

trait ScaleTypes {
    type X;
    type Y;
}

impl ScaleTypes for ScaleProps {
    type X = u32;
    type Y = u32;
}

pub trait ReactorProps {
    type Builder;

    fn builder3(reactor_builder: &mut ReactorBuilderState<'_>) -> Self::Builder;
}

impl ReactorProps for ScaleProps {
    type Builder = ScalePropsBuilder<((), (Input<u32>,), (Output<u32>,))>;
    fn builder3(reactor_builder: &mut ReactorBuilderState<'_>) -> Self::Builder {
        let x = <Input<u32> as boomerang::builder::ReactorField>::build("x", (), reactor_builder)
            .unwrap();
        let y = <Output<u32> as boomerang::builder::ReactorField>::build("y", (), reactor_builder)
            .unwrap();
        Self::builder().x(x).y(y)
    }
}

//#[reactor2]
fn Scale(
    //#[prop(default = 2)]
    //scale: u32,
    //#[input]
    //x: u32,
    //#[output]
    //y: u32,
    props: &ScaleProps,
    mut builder: ReactorBuilderState<'_>,
) -> () {
    let ScaleProps { scale, x, y } = props;

    let scale = *scale;

    /* reaction(x) -> y {} */
    let __reaction = builder
        .add_reaction("", |_| {
            runtime::reaction_closure!(ctx, _reactor, _ref_ports, _mut_ports, _actions => {
                //let x: runtime::InputRef<X> = _ref_ports.partition().unwrap();
                //let mut y: runtime::OutputRef<Y> = _mut_ports.partition_mut().unwrap();
                //*y = Some(2 * x.unwrap());
            })
            .into()
        })
        .with_port(*x, 0, boomerang_builder::TriggerMode::TriggersAndUses)
        .unwrap()
        .with_port(*y, 0, boomerang_builder::TriggerMode::EffectsOnly)
        .unwrap()
        .finish()
        .unwrap();
}

#[derive(::boomerang::typed_builder_macro::TypedBuilder)]
#[builder(crate_module_path = ::boomerang::typed_builder)]
#[allow(non_snake_case)]
struct GainProps {}

impl ReactorProps for GainProps {
    type Builder = GainPropsBuilder<(())>;
    fn builder3(reactor_builder: &mut ReactorBuilderState<'_>) -> Self::Builder {
        Self::builder()
    }
}

fn Gain(props: &GainProps, mut builder: ReactorBuilderState<'_>) -> () {
    //g: Scale,
    //t: Test,
    //tim: Timer,

    /*
    let g = {
        let mut scale_builder =
            builder
                .env()
                .add_reactor("g", Some(builder.key()), None, (), false);
        let scale_props = ScaleProps::builder3(&mut scale_builder).scale(2).build();
        Scale(&scale_props, scale_builder);
        scale_props
    };

    let tim = builder.add_timer("tim", TimerSpec::STARTUP).unwrap();
    */

    /*
    reaction(tim) -> g.x {
    }
    */
}

#[test]
fn gain2() {
    //#[reactor]
    fn Test(
        //#[input] x: u32,
        props: TestProps,
    ) {
    }

    struct TestProps {
        pub x: Input<u32>,
    }
}
