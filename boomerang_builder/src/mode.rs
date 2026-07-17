use variadics_please::all_tuples;

use crate::{
    runtime, ActionTag, PortBank, PortTag, RuntimeAliases, TimerActionKey, TypedActionKey,
    TypedPortKey,
};

slotmap::new_key_type! {
    /// Stable key for an assembly mode specification.
    pub struct AssemblyModeKey;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeKind {
    Initial,
    Normal,
}

impl ModeKind {
    pub fn is_initial(self) -> bool {
        matches!(self, ModeKind::Initial)
    }
}

impl From<bool> for ModeKind {
    fn from(initial: bool) -> Self {
        if initial {
            ModeKind::Initial
        } else {
            ModeKind::Normal
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// Build-time description of a mode transition effect.
pub struct ModeEffectSpec {
    /// Assembly mode targeted by the transition.
    target: AssemblyModeKey,
    /// Runtime mode key resolved during assembly lowering.
    runtime_target: Option<runtime::ModeKey>,
    /// Transition applied when the containing reaction executes.
    transition: runtime::TransitionKind,
}

impl ModeEffectSpec {
    pub(crate) fn new(target: AssemblyModeKey, transition: runtime::TransitionKind) -> Self {
        Self {
            target,
            runtime_target: None,
            transition,
        }
    }

    pub fn target(&self) -> AssemblyModeKey {
        self.target
    }

    pub fn transition(&self) -> runtime::TransitionKind {
        self.transition
    }

    pub fn with_transition(mut self, transition: runtime::TransitionKind) -> Self {
        self.transition = transition;
        self
    }
}

impl runtime::ReactionRefsExtract for ModeEffectSpec {
    type Ref<'store>
        = runtime::ModeEffectRef
    where
        Self: 'store;

    fn extract<'store>(
        &self,
        _refs: &mut runtime::ReactionRefs<'store>,
    ) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let target = self
            .runtime_target
            .ok_or_else(|| runtime::ReactionRefsError::missing("mode effect"))?;
        Ok(runtime::ModeEffectRef::new_key(target, self.transition))
    }
}

#[doc(hidden)]
pub trait ResolveModeEffects {
    fn resolve_mode_effects(&mut self, aliases: &RuntimeAliases);
}

impl ResolveModeEffects for () {
    fn resolve_mode_effects(&mut self, _aliases: &RuntimeAliases) {}
}

impl ResolveModeEffects for ModeEffectSpec {
    fn resolve_mode_effects(&mut self, aliases: &RuntimeAliases) {
        self.runtime_target = Some(aliases.mode_aliases[self.target].1);
    }
}

impl<T: ResolveModeEffects, const N: usize> ResolveModeEffects for [T; N] {
    fn resolve_mode_effects(&mut self, aliases: &RuntimeAliases) {
        for item in self {
            item.resolve_mode_effects(aliases);
        }
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> ResolveModeEffects for TypedActionKey<T, Q> {
    fn resolve_mode_effects(&mut self, _aliases: &RuntimeAliases) {}
}

impl ResolveModeEffects for TimerActionKey {
    fn resolve_mode_effects(&mut self, _aliases: &RuntimeAliases) {}
}

impl<T: runtime::ReactorData, Q: PortTag, A> ResolveModeEffects for TypedPortKey<T, Q, A> {
    fn resolve_mode_effects(&mut self, _aliases: &RuntimeAliases) {}
}

impl<T: runtime::ReactorData, Q: PortTag, A> ResolveModeEffects for PortBank<T, Q, A> {
    fn resolve_mode_effects(&mut self, _aliases: &RuntimeAliases) {}
}

macro_rules! impl_resolve_mode_effects {
    ($($T:ident),*) => {
        impl<$($T,)*> ResolveModeEffects for ($($T,)*)
        where
            $($T: ResolveModeEffects,)*
        {
            #[allow(non_snake_case)]
            fn resolve_mode_effects(&mut self, aliases: &RuntimeAliases) {
                let ($($T,)*) = self;
                $($T.resolve_mode_effects(aliases);)*
            }
        }
    };
}

all_tuples!(impl_resolve_mode_effects, 1, 10, T);
