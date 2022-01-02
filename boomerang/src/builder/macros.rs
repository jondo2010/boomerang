#[macro_export]
macro_rules! ReactorActions {
    ($reactor:ty, $name:ident, $(($var:ident, $typ:ty, $delay:expr)),*) => {
        #[derive(Clone, Default)]
        struct $name {
            $($var: boomerang::runtime::ActionKey,)*
        }
        impl $name {
            fn build(
                env: &mut boomerang::builder::EnvBuilder,
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
