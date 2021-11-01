#[macro_export]
macro_rules! ReactorInputs {
    ($reactor:ty, $name:ident, $(($var:ident, $typ:ty)),*) => {
        #[derive(Clone, Default)]
        struct $name {
            $($var: boomerang::runtime::PortKey,)*
        }
        impl $name {
            fn build<S: boomerang::runtime::SchedulerPoint>(
                env: &mut boomerang::builder::EnvBuilder<S>,
                reactor_key: boomerang::runtime::ReactorKey,
            ) -> Result<Self, boomerang::builder::BuilderError> {
                $(let $var = env.add_port::<$typ>(
                    stringify!($var),
                    ::boomerang::builder::PortType::Input,
                    reactor_key)?;
                )*
                Ok(Self { $($var,)* })
            }
        }
    };
}

#[macro_export]
macro_rules! ReactorOutputs {
    ($reactor:ty, $name:ident, $(($var:ident, $typ:ty)),*) => {
        #[derive(Clone, Default)]
        struct $name {
            $($var: boomerang::runtime::PortKey,)*
        }
        impl $name {
            fn build<S: boomerang::runtime::SchedulerPoint>(
                env: &mut boomerang::builder::EnvBuilder<S>,
                reactor_key: boomerang::runtime::ReactorKey,
            ) -> Result<Self, boomerang::builder::BuilderError> {
                $(let $var = env.add_port::<$typ>(
                    stringify!($var),
                    ::boomerang::builder::PortType::Output,
                    reactor_key)?;
                )*
                Ok(Self { $($var,)* })
            }
        }
    };
}

#[macro_export]
macro_rules! ReactorActions {
    ($reactor:ty, $name:ident, $(($var:ident, $typ:ty, $delay:expr)),*) => {
        #[derive(Clone, Default)]
        struct $name {
            $($var: boomerang::runtime::ActionKey,)*
        }
        impl $name {
            fn build<S: boomerang::runtime::SchedulerPoint>(
                env: &mut boomerang::builder::EnvBuilder<S>,
                reactor_key: boomerang::runtime::ReactorKey,
            ) -> Result<Self, boomerang::builder::BuilderError> {
                $(let $var = env.add_logical_action::<$typ>(
                    stringify!($var),
                    $delay,
                    reactor_key)?;
                )*
                Ok(Self { $($var,)* })
            }
        }
    };
}
