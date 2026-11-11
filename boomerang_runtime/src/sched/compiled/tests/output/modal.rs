//! Validated compiled modal topology for output closure checks.
use super::harness::*;
use crate::image::{ModeImage, TimingDomain};
use crate::{ActionKey, PayloadType, TransitionKind};

static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    IndexSpan::new(0, 2),
    Some(ModeIndex::new(0)),
    None,
)];
static MODES: [ModeImage; 2] = [
    ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(1)),
    ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(2)),
];
static ACTIONS: [ActionImage; 3] = [
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Physical,
            min_delay_nanos: 0,
        },
        SliceRange::new(0, 1),
        Some(BindingSlotIndex::new(4)),
    ),
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(1),
        ActionTiming::Standard {
            domain: TimingDomain::Physical,
            min_delay_nanos: 0,
        },
        SliceRange::new(1, 1),
        Some(BindingSlotIndex::new(4)),
    ),
    ActionImage::new(
        ScopeIndex::new(1),
        ActionSlotIndex::new(2),
        ActionTiming::Standard {
            domain: TimingDomain::Logical,
            min_delay_nanos: 0,
        },
        SliceRange::new(2, 1),
        Some(BindingSlotIndex::new(4)),
    ),
];
const fn reactions(transition: TransitionKind) -> [ReactionImage; 3] {
    [
        ReactionImage::new(
            ReactorIndex::new(0),
            ScopeIndex::new(0),
            0,
            BindingSlotIndex::new(1),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )
        .with_mode_effect(CompiledModeEffectRef {
            target: ModeIndex::new(1),
            transition: TransitionKind::History,
        }),
        ReactionImage::new(
            ReactorIndex::new(0),
            ScopeIndex::new(0),
            0,
            BindingSlotIndex::new(2),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )
        .with_mode_effect(CompiledModeEffectRef {
            target: ModeIndex::new(0),
            transition,
        }),
        ReactionImage::new(
            ReactorIndex::new(0),
            ScopeIndex::new(1),
            0,
            BindingSlotIndex::new(3),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 1),
        )
        .with_external_effect(true),
    ]
}
static HISTORY_REACTIONS: [ReactionImage; 3] = reactions(TransitionKind::History);
static RESET_REACTIONS: [ReactionImage; 3] = reactions(TransitionKind::Reset);
static SCOPES: [ScopeImage; 3] = [
    ScopeImage::new(
        None,
        ReactorIndex::new(0),
        None,
        SliceRange::new(0, 3),
        SliceRange::new(0, 1),
        SliceRange::new(0, 0),
        SliceRange::new(0, 1),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
    ),
    ScopeImage::new(
        Some(ScopeIndex::new(0)),
        ReactorIndex::new(0),
        Some(ModeIndex::new(0)),
        SliceRange::new(3, 1),
        SliceRange::new(1, 1),
        SliceRange::new(0, 0),
        SliceRange::new(1, 1),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
    ),
    ScopeImage::new(
        Some(ScopeIndex::new(0)),
        ReactorIndex::new(0),
        Some(ModeIndex::new(1)),
        SliceRange::new(4, 1),
        SliceRange::new(2, 0),
        SliceRange::new(0, 0),
        SliceRange::new(2, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
    ),
];
static TRIGGERS: [LevelReactionImage; 3] = [
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    LevelReactionImage::new(0, ReactionIndex::new(1)),
    LevelReactionImage::new(0, ReactionIndex::new(2)),
];
static DESCENDANTS: [ScopeIndex; 5] = [
    ScopeIndex::new(0),
    ScopeIndex::new(1),
    ScopeIndex::new(2),
    ScopeIndex::new(1),
    ScopeIndex::new(2),
];
static REQUIRED: [RequiredBindingImage; 5] = [
    RequiredBindingImage::new(
        crate::image::BindingSlotId::new("a-state"),
        BindingKind::StateInitializer,
    ),
    RequiredBindingImage::new(
        crate::image::BindingSlotId::new("b-deactivate"),
        BindingKind::Reaction,
    ),
    RequiredBindingImage::new(
        crate::image::BindingSlotId::new("c-activate"),
        BindingKind::Reaction,
    ),
    RequiredBindingImage::new(
        crate::image::BindingSlotId::new("d-effect"),
        BindingKind::Reaction,
    ),
    RequiredBindingImage::new(
        crate::image::BindingSlotId::new("e-unit"),
        BindingKind::Action,
    ),
];

pub(super) struct ModalFixture {
    pub scheduler: CompiledScheduler,
    pub deactivate: ActionKey,
    pub activate: ActionKey,
    pub pending: ActionKey,
    pub calls: Arc<Mutex<Vec<Tag>>>,
}

pub(super) fn fixture(transition: TransitionKind) -> ModalFixture {
    static FILTER: [ModeIndex; 1] = [ModeIndex::new(0)];
    static LOGICAL: [ActionIndex; 2] = [ActionIndex::new(2), ActionIndex::new(2)];
    static RESETS: [LevelReactionImage; 2] = [
        LevelReactionImage::new(0, ReactionIndex::new(2)),
        LevelReactionImage::new(0, ReactionIndex::new(2)),
    ];
    static BOUNDS: StorageBounds = StorageBounds::new(1, 3, 8, 0, 0, 0);
    let image = EnclaveImage {
        reactors: TinyMapRef::from_slice(&REACTORS),
        actions: TinyMapRef::from_slice(&ACTIONS),
        reactions: TinyMapRef::from_slice(match transition {
            TransitionKind::History => &HISTORY_REACTIONS,
            TransitionKind::Reset => &RESET_REACTIONS,
        }),
        modes: TinyMapRef::from_slice(&MODES),
        scopes: TinyMapRef::from_slice(&SCOPES),
        reaction_triggers: &TRIGGERS,
        reaction_modes: &FILTER,
        scope_descendants: &DESCENDANTS,
        scope_logical_actions: &LOGICAL,
        scope_reset_reactions: &RESETS,
        timer_startup_actions: &[],
        required_bindings: TinyMapRef::from_slice(&REQUIRED),
        storage_bounds: &BOUNDS,
        ..IMAGE.clone()
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let transition = |ctx: &mut Context,
                      _: &mut dyn crate::ReactorData,
                      _: ReactionRefs<'_>,
                      effect: Option<CompiledModeEffectRef>| {
        effect.expect("validated mode effect").set(ctx);
        Ok(())
    };
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(BindingSlotIndex::new(1), transition)
        .bind_reaction(BindingSlotIndex::new(2), transition)
        .bind_reaction(
            BindingSlotIndex::new(3),
            move |ctx: &mut Context, _, _: ReactionRefs<'_>, _| {
                recorded.lock().unwrap().push(ctx.get_tag());
                Ok(OutputOutcome::Committed)
            },
        )
        .bind_action(BindingSlotIndex::new(4), PayloadType::<()>::new());
    let storage = OwnedStorage::new(EnclaveImageView::new(image).unwrap(), bindings).unwrap();
    let deactivate = storage.scheduler_action_ref(ActionIndex::new(0)).key();
    let activate = storage.scheduler_action_ref(ActionIndex::new(1)).key();
    let pending = storage.scheduler_action_ref(ActionIndex::new(2)).key();
    ModalFixture {
        scheduler: CompiledScheduler::new(EnclaveKey::from(1), storage),
        deactivate,
        activate,
        pending,
        calls,
    }
}
