#[macro_export]
macro_rules! ReactorInputs {
    ($name:ident, $(($var:ident, $typ:ty)),*) => {
        #[derive(Clone)]
        struct $name {
            $($var: boomerang::runtime::PortKey<$typ>,)*
        }
        impl boomerang::builder::ReactorPart for $name {
            fn build(
                env: &mut boomerang::builder::EnvBuilder,
                reactor_key: boomerang::runtime::ReactorKey,
            ) -> Result<Self, boomerang::builder::BuilderError> {
                $(let $var = env.add_port::<$typ>(stringify!($var), boomerang::builder::PortType::Input, reactor_key)?;)*
                Ok(Self { $($var,)* })
            }
        }
    };
}

#[macro_export]
macro_rules! ReactorOutputs {
    ($name:ident, $(($var:ident, $typ:ty)),*) => {
        #[derive(Clone)]
        struct $name {
            $($var: boomerang::runtime::PortKey<$typ>,)*
        }
        impl boomerang::builder::ReactorPart for $name {
            fn build(
                env: &mut boomerang::builder::EnvBuilder,
                reactor_key: boomerang::runtime::ReactorKey,
            ) -> Result<Self, boomerang::builder::BuilderError> {
                $(let $var = env.add_port::<$typ>(stringify!($var), boomerang::builder::PortType::Output, reactor_key)?;)*
                Ok(Self { $($var,)* })
            }
        }
    };
}

#[macro_export]
macro_rules! ReactorActions {
    ($name:ident, $(($var:ident, $typ:ty, $delay:expr)),*) => {
        #[derive(Clone)]
        struct $name {
            $($var: boomerang::runtime::ActionKey<$typ>,)*
        }
        impl boomerang::builder::ReactorPart for $name {
            fn build(
                env: &mut boomerang::builder::EnvBuilder,
                reactor_key: boomerang::runtime::ReactorKey,
            ) -> Result<Self, boomerang::builder::BuilderError> {
                $(let $var = env.add_logical_action::<$typ>(stringify!($var), $delay, reactor_key)?;)*
                Ok(Self { $($var,)* })
            }
        }
    };
}