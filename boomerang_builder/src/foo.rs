use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    EnvBuilder, PartialReactionBuilder, PartialReactionBuilderField, ReactionBuilder,
    ReactorBuilderState, TriggerMode,
};
use boomerang_runtime::{Reaction, ReactionRefsExtract};

pub trait ReactorPart {
    const OFFSET: usize;
    type Inner: PartialReactionBuilderField;
}

struct Trig<'a, T: ReactorPart> {
    value: <T::Inner as ReactionRefsExtract>::Ref<'a>,
}

impl<'a, T: ReactorPart> Deref for Trig<'a, T> {
    type Target = T::Inner;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

struct Eff<'a, T> {
    value: &'a mut T,
}

impl<'a, T> Deref for Eff<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T> DerefMut for Eff<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

pub trait IntoReaction<Marker>: Sized {
    type Reaction;
    fn into_system(func: Self) -> Self::Reaction;
}

pub trait ReactionParam: Sized {
    /// Used to store data which persists across invocations of a system.
    type State: Send + Sync + 'static;

    /// The item type returned when constructing this system param.
    /// The value of this associated type should be `Self`, instantiated with new lifetimes.
    ///
    /// You could think of `ReactionParam::Item<'w, 's>` as being an *operation* that changes the lifetimes bound to `Self`.
    type Item<'world, 'state>: ReactionParam<State = Self::State>;

    //unsafe fn get_param<'w, 's>(//&mut component_id: &'s mut Self::State,
    //system_meta: &SystemMeta,
    //world: UnsafeWorldCell<'w>,
    //change_tick: Tick,
    //) -> Self::Item<'w, 's>;

    //fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>( builder: &mut PartialReactionBuilder<S, Fields>,);
}

/// Shorthand way of accessing the associated type [`SystemParam::Item`] for a given [`SystemParam`].
pub type ReactionParamItem<'w, 's, P> = <P as ReactionParam>::Item<'w, 's>;

impl<'a, T: ReactorPart + Send + Sync + 'static> ReactionParam for Trig<'a, T> {
    type State = ();
    type Item<'w, 's> = Trig<'w, T>;

    //#[inline]
    //unsafe fn get_param<'w, 's>(//&mut component_id: &'s mut Self::State,
    //system_meta: &SystemMeta,
    //world: UnsafeWorldCell<'w>,
    //change_tick: Tick,
    //) -> Self::Item<'w, 's> {
    //todo!()
    //}

    //fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
    //    _: &mut PartialReactionBuilder<S, Fields>,
    //) -> () {
    //    //T::Inner::extend_builder_offset(builder, T::OFFSET, TriggerMode::TriggersAndUses);
    //    todo!()
    //}
}

impl<T: ReactionParam> ReactionRefsExtract for T {
    type Ref<'store>
    where
        Self: 'store;

    fn extract<'store>(refs: &mut boomerang_runtime::ReactionRefs<'store>) -> Self::Ref<'store> {
        todo!()
    }
}

/// A trait implemented for all functions that can be used as [`Reaction`]s.
pub trait ReactionParamFunction<Marker>: Send + Sync + 'static {
    /// The [`ReactionParam`]/s used by this system to access the [`World`].
    type Param: ReactionParam;

    fn build(parent_key: BuilderReactorKey) -> ReactionBuilder;
}

macro_rules! impl_reaction_function {
        ($($param: ident),*) => {
            #[allow(non_snake_case)]
            impl<F: Send + Sync + 'static, $($param: ReactionParam),*> ReactionParamFunction<fn($($param,)*)> for F
            where
            for <'a> &'a mut F:
                    FnMut($($param),*) -> () +
                    FnMut($(ReactionParamItem<$param>),*) -> (),
            {
                type Param = ($($param,)*);

                #[inline]
                fn build(parent_key: BuilderReactorKey) -> ReactionBuilder {
                    let builder = ReactionBuilder::new(None, parent_key);
                    builder
                }
            }
        };
    }

impl<F: Send + Sync + 'static, P0: ReactionParam> ReactionParamFunction<fn(P0)> for F
where
    for<'a> &'a mut F: FnMut(P0) -> () + FnMut(ReactionParamItem<P0>) -> (),
{
    type Param = (P0,);
    #[inline]
    fn build(parent_key: BuilderReactorKey) -> ReactionBuilder {
        let builder = ReactionBuilder::new(None, parent_key);
        builder
    }
}

pub struct FunctionReaction<Marker, F>
where
    F: ReactionParamFunction<Marker>,
{
    func: F,
    param_state: Option<<F::Param as ReactionParam>::State>,
    //system_meta: SystemMeta,
    //world_id: Option<WorldId>,
    //archetype_generation: ArchetypeGeneration,
    // NOTE: PhantomData<fn()-> T> gives this safe Send/Sync impls
    marker: PhantomData<fn() -> Marker>,
}

impl<Marker, F> IntoReaction<Marker> for F
where
    Marker: 'static,
    F: ReactionParamFunction<Marker>,
{
    type Reaction = FunctionReaction<Marker, F>;
    fn into_system(func: Self) -> Self::Reaction {
        FunctionReaction {
            func,
            param_state: None,
            marker: PhantomData,
        }
    }
}

mod scale {
    use crate::{foo::ReactorPart, Input, Local, Output, TypedPortKey};

    pub struct X;

    impl ReactorPart for &X {
        const OFFSET: usize = 0;
        type Inner = TypedPortKey<u32, Input, Local>;
    }

    pub struct Y;

    impl ReactorPart for Y {
        const OFFSET: usize = 1;
        type Inner = TypedPortKey<u32, Output, Local>;
    }
}

fn foo<const SCALE: u32>(x: Trig<&scale::X>, mut y: Eff<&mut scale::Y>) {
    // Scale the input value by the specified scale factor
    //*y = Some(SCALE * x);
}

fn test() {
    let mut env = EnvBuilder::new();
    let mut b = env.add_reactor("test", None, None, (), false);

    //<Trig<&scale::X> as ReactionParam>::extend_builder(&mut b)
}
