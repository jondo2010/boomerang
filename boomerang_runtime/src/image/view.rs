use super::*;
use tinymap::{Key, TinyMapView};

/// A precise, allocation-free scheduler-image validation failure.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageValidationError<'a> {
    /// A dense table exceeds its key domain.
    #[error("{table} exceeds its dense key domain")]
    TableTooLarge {
        /// Offending table.
        table: &'static str,
    },
    /// A dense reference is outside its target table.
    #[error("{table}[{index}].{field} references missing {target}[{referenced}]")]
    ReferenceOutOfBounds {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Source field.
        field: &'static str,
        /// Target table.
        target: &'static str,
        /// Invalid target index.
        referenced: u32,
    },
    /// A typed range exceeds its flattened target table.
    #[error("{table}[{index}].{field} range {start}+{len} exceeds {target}")]
    RangeOutOfBounds {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Source field.
        field: &'static str,
        /// Target flattened table.
        target: &'static str,
        /// Invalid range start.
        start: u32,
        /// Invalid range length.
        len: u32,
    },
    /// Canonical owner ranges overlap or move backwards.
    #[error("{table}[{index}].{field} starts at {start} before {previous_end}")]
    RangesNotMonotonic {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Source field.
        field: &'static str,
        /// Offending start.
        start: u32,
        /// End of the previous range.
        previous_end: u32,
    },
    /// Dependency entries are not in canonical order.
    #[error("{table}[{index}] is not sorted")]
    EntriesNotSorted {
        /// Flattened table.
        table: &'static str,
        /// Offending entry index.
        index: u32,
    },
    /// A dependency entry is repeated for one owner.
    #[error("{table}[{index}] duplicates its predecessor")]
    DuplicateEntry {
        /// Flattened table.
        table: &'static str,
        /// Offending entry index.
        index: u32,
    },
    /// A reactor, mode, or scope relationship disagrees.
    #[error("{table}[{index}].{field} has inconsistent ownership")]
    OwnershipMismatch {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Inconsistent relationship.
        field: &'static str,
    },
    /// A stable identity is empty, untrimmed, or contains controls.
    #[error("invalid {kind} identity at {index}: {id:?}")]
    InvalidStableId {
        /// Identity kind.
        kind: &'static str,
        /// Source record index.
        index: u32,
        /// Offending borrowed identity.
        id: &'a str,
    },
    /// Stable identities are not in lexical order.
    #[error("{kind} identity at {index} is not sorted: {id:?}")]
    StableIdsNotSorted {
        /// Identity kind.
        kind: &'static str,
        /// Source record index.
        index: u32,
        /// Offending borrowed identity.
        id: &'a str,
    },
    /// A stable identity duplicates its predecessor.
    #[error("duplicate {kind} identity at {index}: {id:?}")]
    DuplicateStableId {
        /// Identity kind.
        kind: &'static str,
        /// Source record index.
        index: u32,
        /// Offending borrowed identity.
        id: &'a str,
    },
    /// An identity range exceeds the UTF-8 identity blob or splits a character.
    #[error("{table}[{index}].{field} identity range {start}+{len} is invalid")]
    IdentityRangeInvalid {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Identity field.
        field: &'static str,
        /// Invalid byte offset.
        start: u32,
        /// Invalid byte length.
        len: u32,
    },
    /// Reactor-bank metadata has an empty total or an out-of-range index.
    #[error("reactors[{reactor}] bank index {index} is outside total {total}")]
    InvalidBankInfo {
        /// Dense reactor index.
        reactor: u32,
        /// Invalid bank index.
        index: u32,
        /// Declared bank width.
        total: u32,
    },
    /// A dense storage slot reaches or exceeds its declared bound.
    #[error("{table}[{index}] {kind} slot {slot} exceeds bound {bound}")]
    StorageBoundExceeded {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Storage kind.
        kind: &'static str,
        /// Referenced slot.
        slot: u32,
        /// Exclusive bound.
        bound: u32,
    },
    /// A required implementation slot has the wrong binding kind.
    #[error("{table}[{index}].{field} has the wrong binding kind")]
    BindingKindMismatch {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Binding field.
        field: &'static str,
    },
}

/// A validated, allocation-free borrowed view of one Enclave image.
#[derive(Debug)]
pub struct EnclaveImageView<'a> {
    image: &'a EnclaveImage<'a>,
}

impl<'a> EnclaveImageView<'a> {
    /// Validates `image` and borrows all of its tables without copying.
    pub fn new(image: &'a EnclaveImage<'a>) -> Result<Self, ImageValidationError<'a>> {
        validate(image)?;
        Ok(Self { image })
    }
    /// Returns the stable Enclave identity.
    pub fn enclave_id(&self) -> EnclaveId<'a> {
        EnclaveId::new(identity_slice_unchecked(
            self.image.identity_data,
            self.image.enclave_id,
        ))
    }
    /// Returns the dense reactor table.
    pub const fn reactors(&self) -> TinyMapView<'a, ReactorIndex, ReactorImage> {
        TinyMapView::new(self.image.reactors)
    }
    /// Returns the dense action table.
    pub const fn actions(&self) -> TinyMapView<'a, ActionIndex, ActionImage> {
        TinyMapView::new(self.image.actions)
    }
    /// Returns the dense port table.
    pub const fn ports(&self) -> TinyMapView<'a, PortIndex, PortImage> {
        TinyMapView::new(self.image.ports)
    }
    /// Returns the dense reaction table.
    pub const fn reactions(&self) -> TinyMapView<'a, ReactionIndex, ReactionImage> {
        TinyMapView::new(self.image.reactions)
    }
    /// Returns the dense mode table.
    pub const fn modes(&self) -> TinyMapView<'a, ModeIndex, ModeImage> {
        TinyMapView::new(self.image.modes)
    }
    /// Returns the dense scope table.
    pub const fn scopes(&self) -> TinyMapView<'a, ScopeIndex, ScopeImage> {
        TinyMapView::new(self.image.scopes)
    }
    /// Returns the dense boundary-route table.
    pub const fn routes(&self) -> TinyMapView<'a, RouteIndex, RouteImage> {
        TinyMapView::new(self.image.routes)
    }
    /// Returns the dense required-binding table.
    pub const fn required_bindings(
        &self,
    ) -> TinyMapView<'a, BindingSlotIndex, RequiredBindingImage> {
        TinyMapView::new(self.image.required_bindings)
    }
    /// Resolves a route's stable boundary identity.
    pub fn route_boundary_id(&self, key: RouteIndex) -> BoundaryId<'a> {
        BoundaryId::new(identity_slice_unchecked(
            self.image.identity_data,
            self.image.routes[key.as_u32() as usize].boundary(),
        ))
    }
    /// Resolves a required implementation binding's stable identity.
    pub fn required_binding_id(&self, key: BindingSlotIndex) -> BindingSlotId<'a> {
        BindingSlotId::new(identity_slice_unchecked(
            self.image.identity_data,
            self.image.required_bindings[key.as_u32() as usize].id(),
        ))
    }
    /// Returns the declared mutable-storage and workspace bounds.
    pub const fn storage_bounds(&self) -> StorageBounds {
        self.image.storage_bounds
    }
    /// Returns an action's ordered leveled triggers.
    pub fn action_triggers(&self, key: ActionIndex) -> &'a [LevelReactionImage] {
        range_slice(
            self.image.actions[key.as_u32() as usize].triggers(),
            self.image.reaction_triggers,
        )
    }
    /// Returns a port's ordered leveled triggers.
    pub fn port_triggers(&self, key: PortIndex) -> &'a [LevelReactionImage] {
        range_slice(
            self.image.ports[key.as_u32() as usize].triggers(),
            self.image.reaction_triggers,
        )
    }
    /// Returns a reaction's ordered use ports.
    pub fn reaction_use_ports(&self, key: ReactionIndex) -> &'a [PortIndex] {
        range_slice(
            self.image.reactions[key.as_u32() as usize].use_ports(),
            self.image.reaction_use_ports,
        )
    }
    /// Returns a reaction's ordered effect ports.
    pub fn reaction_effect_ports(&self, key: ReactionIndex) -> &'a [PortIndex] {
        range_slice(
            self.image.reactions[key.as_u32() as usize].effect_ports(),
            self.image.reaction_effect_ports,
        )
    }
    /// Returns a reaction's ordered action references.
    pub fn reaction_actions(&self, key: ReactionIndex) -> &'a [ActionIndex] {
        range_slice(
            self.image.reactions[key.as_u32() as usize].actions(),
            self.image.reaction_actions,
        )
    }
    /// Returns a reaction's enabled modes.
    pub fn reaction_modes(&self, key: ReactionIndex) -> &'a [ModeIndex] {
        range_slice(
            self.image.reactions[key.as_u32() as usize].enabled_modes(),
            self.image.reaction_modes,
        )
    }
    /// Returns a scope's precomputed descendants.
    pub fn scope_descendants(&self, key: ScopeIndex) -> &'a [ScopeIndex] {
        range_slice(
            self.image.scopes[key.as_u32() as usize].descendants(),
            self.image.scope_descendants,
        )
    }
    /// Returns a scope's precomputed logical actions.
    pub fn scope_logical_actions(&self, key: ScopeIndex) -> &'a [ActionIndex] {
        range_slice(
            self.image.scopes[key.as_u32() as usize].logical_actions(),
            self.image.scope_logical_actions,
        )
    }
    /// Returns a scope's precomputed timer startups.
    pub fn scope_timer_startups(&self, key: ScopeIndex) -> &'a [TimerStartupImage] {
        range_slice(
            self.image.scopes[key.as_u32() as usize].timer_startups(),
            self.image.scope_timer_startups,
        )
    }
    /// Returns a scope's precomputed reset reactions.
    pub fn scope_reset_reactions(&self, key: ScopeIndex) -> &'a [LevelReactionImage] {
        range_slice(
            self.image.scopes[key.as_u32() as usize].reset_reactions(),
            self.image.scope_reset_reactions,
        )
    }
    /// Returns a scope's precomputed startup reactions.
    pub fn scope_startup_reactions(&self, key: ScopeIndex) -> &'a [LifecycleReactionImage] {
        range_slice(
            self.image.scopes[key.as_u32() as usize].startup_reactions(),
            self.image.scope_startup_reactions,
        )
    }
    /// Returns a scope's precomputed shutdown reactions.
    pub fn scope_shutdown_reactions(&self, key: ScopeIndex) -> &'a [LifecycleReactionImage] {
        range_slice(
            self.image.scopes[key.as_u32() as usize].shutdown_reactions(),
            self.image.scope_shutdown_reactions,
        )
    }
    /// Returns global startup action entries.
    pub const fn startup_actions(&self) -> &'a [TimerStartupImage] {
        self.image.startup_actions
    }
    /// Returns global timer startup entries.
    pub const fn timer_startup_actions(&self) -> &'a [TimerStartupImage] {
        self.image.timer_startup_actions
    }
    /// Returns global shutdown reaction entries.
    pub const fn shutdown_reactions(&self) -> &'a [LifecycleReactionImage] {
        self.image.shutdown_reactions
    }
    /// Returns unique actions populated before global shutdown reactions.
    pub const fn shutdown_actions(&self) -> &'a [ActionIndex] {
        self.image.shutdown_actions
    }
}

fn check_len<K: Key>(table: &'static str, len: usize) -> Result<(), ImageValidationError<'static>> {
    if len > K::MAX_LEN {
        Err(ImageValidationError::TableTooLarge { table })
    } else {
        Ok(())
    }
}

fn check_ref<'a>(
    table: &'static str,
    index: u32,
    field: &'static str,
    target: &'static str,
    value: u32,
    len: usize,
) -> Result<(), ImageValidationError<'a>> {
    if value as usize >= len {
        Err(ImageValidationError::ReferenceOutOfBounds {
            table,
            index,
            field,
            target,
            referenced: value,
        })
    } else {
        Ok(())
    }
}

fn check_range<'a, K>(
    table: &'static str,
    index: u32,
    field: &'static str,
    target: &'static str,
    range: TableRange<K>,
    len: usize,
    previous_end: &mut u32,
) -> Result<(), ImageValidationError<'a>> {
    let end = range.start().checked_add(range.len());
    if end.map(|end| end as usize <= len) != Some(true) {
        return Err(ImageValidationError::RangeOutOfBounds {
            table,
            index,
            field,
            target,
            start: range.start(),
            len: range.len(),
        });
    }
    if range.start() < *previous_end {
        return Err(ImageValidationError::RangesNotMonotonic {
            table,
            index,
            field,
            start: range.start(),
            previous_end: *previous_end,
        });
    }
    *previous_end = end.unwrap();
    Ok(())
}

fn range_slice<T, K>(range: TableRange<K>, values: &[T]) -> &[T] {
    let start = range.start() as usize;
    &values[start..start + range.len() as usize]
}

fn identity_slice_unchecked(value: &str, range: IdentityRange) -> &str {
    let start = range.start() as usize;
    &value[start..start + range.len() as usize]
}

fn identity_slice<'a>(
    value: &'a str,
    table: &'static str,
    index: u32,
    field: &'static str,
    range: IdentityRange,
) -> Result<&'a str, ImageValidationError<'a>> {
    let Some(end) = range.start().checked_add(range.len()) else {
        return Err(ImageValidationError::IdentityRangeInvalid {
            table,
            index,
            field,
            start: range.start(),
            len: range.len(),
        });
    };
    value.get(range.start() as usize..end as usize).ok_or(
        ImageValidationError::IdentityRangeInvalid {
            table,
            index,
            field,
            start: range.start(),
            len: range.len(),
        },
    )
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

fn validate_id<'a>(
    kind: &'static str,
    index: u32,
    id: &'a str,
    previous: &mut Option<&'a str>,
) -> Result<(), ImageValidationError<'a>> {
    if !valid_id(id) {
        return Err(ImageValidationError::InvalidStableId { kind, index, id });
    }
    if let Some(before) = *previous {
        if id == before {
            return Err(ImageValidationError::DuplicateStableId { kind, index, id });
        }
        if id < before {
            return Err(ImageValidationError::StableIdsNotSorted { kind, index, id });
        }
    }
    *previous = Some(id);
    Ok(())
}

fn validate_level_ref<'a>(
    table: &'static str,
    index: u32,
    entry: LevelReactionImage,
    image: &EnclaveImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    check_ref(
        table,
        index,
        "reaction",
        "reactions",
        entry.reaction().as_u32(),
        image.reactions.len(),
    )?;
    if image.reactions[entry.reaction().as_u32() as usize].dependency_level() != entry.level() {
        return Err(ImageValidationError::OwnershipMismatch {
            table,
            index,
            field: "dependency_level",
        });
    }
    Ok(())
}

fn validate_levels<'a>(
    table: &'static str,
    offset: u32,
    values: &[LevelReactionImage],
    image: &EnclaveImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    let mut previous = None;
    for (position, entry) in values.iter().copied().enumerate() {
        let index = offset + position as u32;
        validate_level_ref(table, index, entry, image)?;
        if let Some(before) = previous {
            if entry == before {
                return Err(ImageValidationError::DuplicateEntry { table, index });
            }
            if entry < before {
                return Err(ImageValidationError::EntriesNotSorted { table, index });
            }
        }
        previous = Some(entry);
    }
    Ok(())
}

fn validate_lifecycle<'a>(
    table: &'static str,
    offset: u32,
    values: &[LifecycleReactionImage],
    image: &EnclaveImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    let mut previous = None;
    for (position, entry) in values.iter().copied().enumerate() {
        let index = offset + position as u32;
        check_ref(
            table,
            index,
            "action",
            "actions",
            entry.action().as_u32(),
            image.actions.len(),
        )?;
        validate_level_ref(table, index, entry.reaction(), image)?;
        if let Some(before) = previous {
            if entry.reaction() == before {
                return Err(ImageValidationError::DuplicateEntry { table, index });
            }
            if entry.reaction() < before {
                return Err(ImageValidationError::EntriesNotSorted { table, index });
            }
        }
        previous = Some(entry.reaction());
    }
    Ok(())
}

fn validate<'a>(image: &EnclaveImage<'a>) -> Result<(), ImageValidationError<'a>> {
    check_len::<ReactorIndex>("reactors", image.reactors.len())?;
    check_len::<ActionIndex>("actions", image.actions.len())?;
    check_len::<PortIndex>("ports", image.ports.len())?;
    check_len::<ReactionIndex>("reactions", image.reactions.len())?;
    check_len::<ModeIndex>("modes", image.modes.len())?;
    check_len::<ScopeIndex>("scopes", image.scopes.len())?;
    check_len::<RouteIndex>("routes", image.routes.len())?;
    check_len::<BindingSlotIndex>("required_bindings", image.required_bindings.len())?;
    let enclave_id = identity_slice(
        image.identity_data,
        "image",
        0,
        "enclave_id",
        image.enclave_id,
    )?;
    validate_id("enclave", 0, enclave_id, &mut None)?;

    let mut mode_end = 0;
    for (i, reactor) in image.reactors.iter().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "reactors",
            index,
            "state_binding",
            "required_bindings",
            reactor.state_binding().as_u32(),
            image.required_bindings.len(),
        )?;
        if image.required_bindings[reactor.state_binding().as_u32() as usize].kind()
            != BindingKind::StateInitializer
        {
            return Err(ImageValidationError::BindingKindMismatch {
                table: "reactors",
                index,
                field: "state_binding",
            });
        }
        if reactor.state_slot().as_u32() >= image.storage_bounds.state_slots() {
            return Err(ImageValidationError::StorageBoundExceeded {
                table: "reactors",
                index,
                kind: "state slots",
                slot: reactor.state_slot().as_u32(),
                bound: image.storage_bounds.state_slots(),
            });
        }
        check_ref(
            "reactors",
            index,
            "root_scope",
            "scopes",
            reactor.root_scope().as_u32(),
            image.scopes.len(),
        )?;
        let root_scope = image.scopes[reactor.root_scope().as_u32() as usize];
        if root_scope.reactor() != ReactorIndex::new(index) || root_scope.mode().is_some() {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "reactors",
                index,
                field: "root_scope",
            });
        }
        check_range(
            "reactors",
            index,
            "modes",
            "modes",
            reactor.modes(),
            image.modes.len(),
            &mut mode_end,
        )?;
        if let Some(mode) = reactor.initial_mode() {
            check_ref(
                "reactors",
                index,
                "initial_mode",
                "modes",
                mode.as_u32(),
                image.modes.len(),
            )?;
            if mode.as_u32() < reactor.modes().start()
                || mode.as_u32() >= reactor.modes().start() + reactor.modes().len()
            {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "reactors",
                    index,
                    field: "initial_mode",
                });
            }
        }
        if let Some(bank) = reactor.bank() {
            if bank.total() == 0 || bank.index() >= bank.total() {
                return Err(ImageValidationError::InvalidBankInfo {
                    reactor: index,
                    index: bank.index(),
                    total: bank.total(),
                });
            }
        }
    }

    let mut trigger_end = 0;
    for (i, action) in image.actions.iter().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "actions",
            index,
            "scope",
            "scopes",
            action.scope().as_u32(),
            image.scopes.len(),
        )?;
        if action.storage_slot().as_u32() >= image.storage_bounds.action_slots() {
            return Err(ImageValidationError::StorageBoundExceeded {
                table: "actions",
                index,
                kind: "action slots",
                slot: action.storage_slot().as_u32(),
                bound: image.storage_bounds.action_slots(),
            });
        }
        check_range(
            "actions",
            index,
            "triggers",
            "reaction_triggers",
            action.triggers(),
            image.reaction_triggers.len(),
            &mut trigger_end,
        )?;
    }
    trigger_end = 0;
    for (i, port) in image.ports.iter().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "ports",
            index,
            "scope",
            "scopes",
            port.scope().as_u32(),
            image.scopes.len(),
        )?;
        check_range(
            "ports",
            index,
            "triggers",
            "reaction_triggers",
            port.triggers(),
            image.reaction_triggers.len(),
            &mut trigger_end,
        )?;
    }

    let (mut use_end, mut effect_end, mut action_end, mut reaction_mode_end) = (0, 0, 0, 0);
    for (i, reaction) in image.reactions.iter().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "reactions",
            index,
            "reactor",
            "reactors",
            reaction.reactor().as_u32(),
            image.reactors.len(),
        )?;
        check_ref(
            "reactions",
            index,
            "scope",
            "scopes",
            reaction.scope().as_u32(),
            image.scopes.len(),
        )?;
        check_ref(
            "reactions",
            index,
            "binding",
            "required_bindings",
            reaction.binding().as_u32(),
            image.required_bindings.len(),
        )?;
        if image.required_bindings[reaction.binding().as_u32() as usize].kind()
            != BindingKind::Reaction
        {
            return Err(ImageValidationError::BindingKindMismatch {
                table: "reactions",
                index,
                field: "binding",
            });
        }
        if image.scopes[reaction.scope().as_u32() as usize].reactor() != reaction.reactor() {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "reactions",
                index,
                field: "scope.reactor",
            });
        }
        check_range(
            "reactions",
            index,
            "use_ports",
            "reaction_use_ports",
            reaction.use_ports(),
            image.reaction_use_ports.len(),
            &mut use_end,
        )?;
        check_range(
            "reactions",
            index,
            "effect_ports",
            "reaction_effect_ports",
            reaction.effect_ports(),
            image.reaction_effect_ports.len(),
            &mut effect_end,
        )?;
        check_range(
            "reactions",
            index,
            "actions",
            "reaction_actions",
            reaction.actions(),
            image.reaction_actions.len(),
            &mut action_end,
        )?;
        check_range(
            "reactions",
            index,
            "enabled_modes",
            "reaction_modes",
            reaction.enabled_modes(),
            image.reaction_modes.len(),
            &mut reaction_mode_end,
        )?;
    }

    for (i, mode) in image.modes.iter().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "modes",
            index,
            "reactor",
            "reactors",
            mode.reactor().as_u32(),
            image.reactors.len(),
        )?;
        check_ref(
            "modes",
            index,
            "scope",
            "scopes",
            mode.scope().as_u32(),
            image.scopes.len(),
        )?;
        let scope = image.scopes[mode.scope().as_u32() as usize];
        if scope.reactor() != mode.reactor() {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "modes",
                index,
                field: "scope.reactor",
            });
        }
        if scope.mode() != Some(ModeIndex::new(index)) {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "modes",
                index,
                field: "scope.mode",
            });
        }
        let modes = image.reactors[mode.reactor().as_u32() as usize].modes();
        if index < modes.start() || index >= modes.start() + modes.len() {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "modes",
                index,
                field: "reactor.modes",
            });
        }
    }

    let mut ends = [0_u32; 6];
    for (i, scope) in image.scopes.iter().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "scopes",
            index,
            "reactor",
            "reactors",
            scope.reactor().as_u32(),
            image.reactors.len(),
        )?;
        if let Some(parent) = scope.parent() {
            check_ref(
                "scopes",
                index,
                "parent",
                "scopes",
                parent.as_u32(),
                image.scopes.len(),
            )?;
        }
        if let Some(mode) = scope.mode() {
            check_ref(
                "scopes",
                index,
                "mode",
                "modes",
                mode.as_u32(),
                image.modes.len(),
            )?;
            let owner = image.modes[mode.as_u32() as usize];
            if owner.scope() != ScopeIndex::new(index) || owner.reactor() != scope.reactor() {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "scopes",
                    index,
                    field: "mode",
                });
            }
        }
        check_range(
            "scopes",
            index,
            "descendants",
            "scope_descendants",
            scope.descendants(),
            image.scope_descendants.len(),
            &mut ends[0],
        )?;
        check_range(
            "scopes",
            index,
            "logical_actions",
            "scope_logical_actions",
            scope.logical_actions(),
            image.scope_logical_actions.len(),
            &mut ends[1],
        )?;
        check_range(
            "scopes",
            index,
            "timer_startups",
            "scope_timer_startups",
            scope.timer_startups(),
            image.scope_timer_startups.len(),
            &mut ends[2],
        )?;
        check_range(
            "scopes",
            index,
            "reset_reactions",
            "scope_reset_reactions",
            scope.reset_reactions(),
            image.scope_reset_reactions.len(),
            &mut ends[3],
        )?;
        check_range(
            "scopes",
            index,
            "startup_reactions",
            "scope_startup_reactions",
            scope.startup_reactions(),
            image.scope_startup_reactions.len(),
            &mut ends[4],
        )?;
        check_range(
            "scopes",
            index,
            "shutdown_reactions",
            "scope_shutdown_reactions",
            scope.shutdown_reactions(),
            image.scope_shutdown_reactions.len(),
            &mut ends[5],
        )?;
    }

    for action in image.actions.iter().copied() {
        validate_levels(
            "reaction_triggers",
            action.triggers().start(),
            range_slice(action.triggers(), image.reaction_triggers),
            image,
        )?;
    }
    for port in image.ports.iter().copied() {
        validate_levels(
            "reaction_triggers",
            port.triggers().start(),
            range_slice(port.triggers(), image.reaction_triggers),
            image,
        )?;
    }
    for (i, value) in image.reaction_use_ports.iter().enumerate() {
        check_ref(
            "reaction_use_ports",
            i as u32,
            "port",
            "ports",
            value.as_u32(),
            image.ports.len(),
        )?;
    }
    for (i, value) in image.reaction_effect_ports.iter().enumerate() {
        check_ref(
            "reaction_effect_ports",
            i as u32,
            "port",
            "ports",
            value.as_u32(),
            image.ports.len(),
        )?;
    }
    for (i, value) in image.reaction_actions.iter().enumerate() {
        check_ref(
            "reaction_actions",
            i as u32,
            "action",
            "actions",
            value.as_u32(),
            image.actions.len(),
        )?;
    }
    for (i, value) in image.reaction_modes.iter().enumerate() {
        check_ref(
            "reaction_modes",
            i as u32,
            "mode",
            "modes",
            value.as_u32(),
            image.modes.len(),
        )?;
    }
    for (i, reaction) in image.reactions.iter().copied().enumerate() {
        for mode in range_slice(reaction.enabled_modes(), image.reaction_modes) {
            if image.modes[mode.as_u32() as usize].reactor() != reaction.reactor() {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "reactions",
                    index: i as u32,
                    field: "enabled_modes.reactor",
                });
            }
        }
    }
    for (i, value) in image.scope_descendants.iter().enumerate() {
        check_ref(
            "scope_descendants",
            i as u32,
            "scope",
            "scopes",
            value.as_u32(),
            image.scopes.len(),
        )?;
    }
    for (i, value) in image.scope_logical_actions.iter().enumerate() {
        check_ref(
            "scope_logical_actions",
            i as u32,
            "action",
            "actions",
            value.as_u32(),
            image.actions.len(),
        )?;
    }
    for (i, value) in image.scope_timer_startups.iter().enumerate() {
        check_ref(
            "scope_timer_startups",
            i as u32,
            "action",
            "actions",
            value.action().as_u32(),
            image.actions.len(),
        )?;
    }
    for scope in image.scopes.iter().copied() {
        validate_levels(
            "scope_reset_reactions",
            scope.reset_reactions().start(),
            range_slice(scope.reset_reactions(), image.scope_reset_reactions),
            image,
        )?;
        validate_lifecycle(
            "scope_startup_reactions",
            scope.startup_reactions().start(),
            range_slice(scope.startup_reactions(), image.scope_startup_reactions),
            image,
        )?;
        validate_lifecycle(
            "scope_shutdown_reactions",
            scope.shutdown_reactions().start(),
            range_slice(scope.shutdown_reactions(), image.scope_shutdown_reactions),
            image,
        )?;
    }
    for (i, entry) in image.reaction_triggers.iter().copied().enumerate() {
        validate_level_ref("reaction_triggers", i as u32, entry, image)?;
    }
    for (i, entry) in image.scope_reset_reactions.iter().copied().enumerate() {
        validate_level_ref("scope_reset_reactions", i as u32, entry, image)?;
    }
    for (table, entries) in [
        ("scope_startup_reactions", image.scope_startup_reactions),
        ("scope_shutdown_reactions", image.scope_shutdown_reactions),
    ] {
        for (i, entry) in entries.iter().copied().enumerate() {
            check_ref(
                table,
                i as u32,
                "action",
                "actions",
                entry.action().as_u32(),
                image.actions.len(),
            )?;
            validate_level_ref(table, i as u32, entry.reaction(), image)?;
        }
    }
    for (i, value) in image.startup_actions.iter().enumerate() {
        check_ref(
            "startup_actions",
            i as u32,
            "action",
            "actions",
            value.action().as_u32(),
            image.actions.len(),
        )?;
    }
    for (i, value) in image.timer_startup_actions.iter().enumerate() {
        check_ref(
            "timer_startup_actions",
            i as u32,
            "action",
            "actions",
            value.action().as_u32(),
            image.actions.len(),
        )?;
    }
    validate_lifecycle("shutdown_reactions", 0, image.shutdown_reactions, image)?;
    let mut previous_action = None;
    for (i, action) in image.shutdown_actions.iter().copied().enumerate() {
        check_ref(
            "shutdown_actions",
            i as u32,
            "action",
            "actions",
            action.as_u32(),
            image.actions.len(),
        )?;
        if let Some(previous) = previous_action {
            if action == previous {
                return Err(ImageValidationError::DuplicateEntry {
                    table: "shutdown_actions",
                    index: i as u32,
                });
            }
            if action < previous {
                return Err(ImageValidationError::EntriesNotSorted {
                    table: "shutdown_actions",
                    index: i as u32,
                });
            }
        }
        previous_action = Some(action);
    }

    let mut previous = None;
    for (i, route) in image.routes.iter().copied().enumerate() {
        let id = identity_slice(
            image.identity_data,
            "routes",
            i as u32,
            "boundary",
            route.boundary(),
        )?;
        validate_id("boundary", i as u32, id, &mut previous)?;
        check_ref(
            "routes",
            i as u32,
            "local_port",
            "ports",
            route.local_port().as_u32(),
            image.ports.len(),
        )?;
    }
    previous = None;
    for (i, binding) in image.required_bindings.iter().copied().enumerate() {
        let id = identity_slice(
            image.identity_data,
            "required_bindings",
            i as u32,
            "id",
            binding.id(),
        )?;
        validate_id("binding", i as u32, id, &mut previous)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::*;

    const RANGE_0_0: TableRange<PortIndex> = TableRange::new(0, 0);

    static REACTORS: [ReactorImage; 2] = [
        ReactorImage::new(
            BindingSlotIndex::new(2),
            StateSlotIndex::new(0),
            ScopeIndex::new(0),
            TableRange::new(0, 1),
            Some(ModeIndex::new(0)),
            Some(BankInfoImage::new(0, 2)),
        ),
        ReactorImage::new(
            BindingSlotIndex::new(3),
            StateSlotIndex::new(1),
            ScopeIndex::new(2),
            TableRange::new(1, 0),
            None,
            Some(BankInfoImage::new(1, 2)),
        ),
    ];
    static ACTIONS: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(1),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Logical,
            min_delay_nanos: 7,
        },
        TableRange::new(0, 2),
    )];
    static PORTS: [PortImage; 2] = [
        PortImage::new(ScopeIndex::new(0), TableRange::new(2, 1)),
        PortImage::new(ScopeIndex::new(2), TableRange::new(3, 1)),
    ];
    static REACTIONS: [ReactionImage; 2] = [
        ReactionImage::new(
            ReactorIndex::new(0),
            ScopeIndex::new(1),
            0,
            BindingSlotIndex::new(0),
            TableRange::new(0, 1),
            TableRange::new(0, 1),
            TableRange::new(0, 1),
            TableRange::new(0, 1),
        ),
        ReactionImage::new(
            ReactorIndex::new(1),
            ScopeIndex::new(2),
            1,
            BindingSlotIndex::new(1),
            TableRange::new(1, 1),
            TableRange::new(1, 0),
            TableRange::new(1, 0),
            TableRange::new(1, 0),
        ),
    ];
    static MODES: [ModeImage; 1] = [ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(1))];
    static SCOPES: [ScopeImage; 3] = [
        ScopeImage::new(
            None,
            ReactorIndex::new(0),
            None,
            TableRange::new(0, 2),
            TableRange::new(0, 1),
            TableRange::new(0, 1),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
        ),
        ScopeImage::new(
            Some(ScopeIndex::new(0)),
            ReactorIndex::new(0),
            Some(ModeIndex::new(0)),
            TableRange::new(2, 1),
            TableRange::new(1, 1),
            TableRange::new(1, 1),
            TableRange::new(0, 1),
            TableRange::new(0, 1),
            TableRange::new(0, 1),
        ),
        ScopeImage::new(
            None,
            ReactorIndex::new(1),
            None,
            TableRange::new(3, 1),
            TableRange::new(2, 0),
            TableRange::new(2, 0),
            TableRange::new(1, 0),
            TableRange::new(1, 0),
            TableRange::new(1, 0),
        ),
    ];
    static REACTION_TRIGGERS: [LevelReactionImage; 4] = [
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        LevelReactionImage::new(1, ReactionIndex::new(1)),
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        LevelReactionImage::new(1, ReactionIndex::new(1)),
    ];
    static REACTION_USE_PORTS: [PortIndex; 2] = [PortIndex::new(0), PortIndex::new(1)];
    static REACTION_EFFECT_PORTS: [PortIndex; 1] = [PortIndex::new(1)];
    static REACTION_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
    static REACTION_MODES: [ModeIndex; 1] = [ModeIndex::new(0)];
    static SCOPE_DESCENDANTS: [ScopeIndex; 4] = [
        ScopeIndex::new(0),
        ScopeIndex::new(1),
        ScopeIndex::new(1),
        ScopeIndex::new(2),
    ];
    static SCOPE_LOGICAL_ACTIONS: [ActionIndex; 2] = [ActionIndex::new(0), ActionIndex::new(0)];
    static SCOPE_TIMER_STARTUPS: [TimerStartupImage; 2] = [
        TimerStartupImage::new(ActionIndex::new(0), 5),
        TimerStartupImage::new(ActionIndex::new(0), 5),
    ];
    static SCOPE_RESET_REACTIONS: [LevelReactionImage; 1] =
        [LevelReactionImage::new(0, ReactionIndex::new(0))];
    static SCOPE_STARTUP_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        ActionIndex::new(0),
    )];
    static SCOPE_SHUTDOWN_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        ActionIndex::new(0),
    )];
    static STARTUP_ACTIONS: [TimerStartupImage; 1] =
        [TimerStartupImage::new(ActionIndex::new(0), 0)];
    static TIMER_STARTUP_ACTIONS: [TimerStartupImage; 1] =
        [TimerStartupImage::new(ActionIndex::new(0), 5)];
    static SHUTDOWN_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
        LevelReactionImage::new(1, ReactionIndex::new(1)),
        ActionIndex::new(0),
    )];
    static SHUTDOWN_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
    static IDENTITY_DATA: &str = "plant/controlnetwork/inreaction/r0reaction/r1state/r0state/r1";
    static ROUTES: [RouteImage; 1] = [RouteImage::new(
        IdentityRange::new(13, 10),
        PortIndex::new(1),
        RouteDirection::Inbound,
        TimingDomain::Physical,
        10,
    )];
    static REQUIRED_BINDINGS: [RequiredBindingImage; 4] = [
        RequiredBindingImage::new(IdentityRange::new(23, 11), BindingKind::Reaction),
        RequiredBindingImage::new(IdentityRange::new(34, 11), BindingKind::Reaction),
        RequiredBindingImage::new(IdentityRange::new(45, 8), BindingKind::StateInitializer),
        RequiredBindingImage::new(IdentityRange::new(53, 8), BindingKind::StateInitializer),
    ];

    static IMAGE: EnclaveImage<'static> = EnclaveImage::new(
        IDENTITY_DATA,
        IdentityRange::new(0, 13),
        &REACTORS,
        &ACTIONS,
        &PORTS,
        &REACTIONS,
        &MODES,
        &SCOPES,
        &REACTION_TRIGGERS,
        &REACTION_USE_PORTS,
        &REACTION_EFFECT_PORTS,
        &REACTION_ACTIONS,
        &REACTION_MODES,
        &SCOPE_DESCENDANTS,
        &SCOPE_LOGICAL_ACTIONS,
        &SCOPE_TIMER_STARTUPS,
        &SCOPE_RESET_REACTIONS,
        &SCOPE_STARTUP_REACTIONS,
        &SCOPE_SHUTDOWN_REACTIONS,
        &STARTUP_ACTIONS,
        &TIMER_STARTUP_ACTIONS,
        &SHUTDOWN_REACTIONS,
        &SHUTDOWN_ACTIONS,
        &ROUTES,
        &REQUIRED_BINDINGS,
        StorageBounds::new(2, 1, 8, 4),
    );

    #[test]
    fn static_scheduler_image_exposes_borrowed_typed_tables() {
        let view = EnclaveImageView::new(&IMAGE).unwrap();

        assert_eq!(view.enclave_id().as_str(), "plant/control");
        assert_eq!(view.reactors().len(), 2);
        assert_eq!(
            view.reactors()[ReactorIndex::new(1)].bank(),
            Some(BankInfoImage::new(1, 2))
        );
        assert_eq!(view.actions().len(), 1);
        assert_eq!(
            view.actions()[ActionIndex::new(0)].timing(),
            ActionTiming::Standard {
                domain: TimingDomain::Logical,
                min_delay_nanos: 7,
            }
        );
        assert_eq!(view.ports().len(), 2);
        assert_eq!(
            view.reactions()[ReactionIndex::new(1)].dependency_level(),
            1
        );
        assert_eq!(
            view.reactions()[ReactionIndex::new(1)].binding(),
            BindingSlotIndex::new(1)
        );
        assert_eq!(
            view.action_triggers(ActionIndex::new(0)),
            &[
                LevelReactionImage::new(0, ReactionIndex::new(0)),
                LevelReactionImage::new(1, ReactionIndex::new(1)),
            ]
        );
        assert_eq!(
            view.route_boundary_id(RouteIndex::new(0)).as_str(),
            "network/in"
        );
        assert_eq!(
            view.routes()[RouteIndex::new(0)].timing_domain(),
            TimingDomain::Physical
        );
        assert_eq!(view.shutdown_actions(), &SHUTDOWN_ACTIONS);
        assert_eq!(
            view.required_binding_id(BindingSlotIndex::new(2)).as_str(),
            "state/r0"
        );
        assert_eq!(view.storage_bounds().event_capacity(), 8);
    }

    #[test]
    fn action_timing_preserves_standard_timer_and_shutdown_semantics() {
        let cases = [
            ActionTiming::Standard {
                domain: TimingDomain::Physical,
                min_delay_nanos: 19,
            },
            ActionTiming::Timer {
                period_nanos: Some(23),
            },
            ActionTiming::Shutdown,
        ];

        for timing in cases {
            let actions = [ActionImage::new(
                ScopeIndex::new(1),
                ActionSlotIndex::new(0),
                timing,
                TableRange::new(0, 2),
            )];
            let image = EnclaveImage {
                actions: &actions,
                ..IMAGE
            };
            let view = EnclaveImageView::new(&image).unwrap();
            assert_eq!(view.actions()[ActionIndex::new(0)].timing(), timing);
        }
    }

    #[test]
    fn invalid_cross_references_and_ranges_report_the_source_location() {
        let bad_reactions = [ReactionImage::new(
            ReactorIndex::new(9),
            ScopeIndex::new(1),
            0,
            BindingSlotIndex::new(0),
            RANGE_0_0,
            RANGE_0_0,
            TableRange::new(0, 0),
            TableRange::new(0, 0),
        )];
        let bad_actions = [ActionImage::new(
            ScopeIndex::new(1),
            ActionSlotIndex::new(0),
            ActionTiming::Standard {
                domain: TimingDomain::Logical,
                min_delay_nanos: 7,
            },
            TableRange::new(4, 1),
        )];
        let cases = [
            (
                "identity range",
                EnclaveImage {
                    enclave_id: IdentityRange::new(u32::MAX, 2),
                    ..IMAGE
                },
                ImageValidationError::IdentityRangeInvalid {
                    table: "image",
                    index: 0,
                    field: "enclave_id",
                    start: u32::MAX,
                    len: 2,
                },
            ),
            (
                "primary cross-reference",
                EnclaveImage {
                    reactions: &bad_reactions,
                    ..IMAGE
                },
                ImageValidationError::ReferenceOutOfBounds {
                    table: "reactions",
                    index: 0,
                    field: "reactor",
                    target: "reactors",
                    referenced: 9,
                },
            ),
            (
                "flattened range",
                EnclaveImage {
                    actions: &bad_actions,
                    ..IMAGE
                },
                ImageValidationError::RangeOutOfBounds {
                    table: "actions",
                    index: 0,
                    field: "triggers",
                    target: "reaction_triggers",
                    start: 4,
                    len: 1,
                },
            ),
        ];

        for (name, image, expected) in cases {
            assert_eq!(
                EnclaveImageView::new(&image).unwrap_err(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn invalid_trigger_order_and_duplicates_report_the_flattened_entry() {
        let unsorted = [
            LevelReactionImage::new(1, ReactionIndex::new(1)),
            LevelReactionImage::new(0, ReactionIndex::new(0)),
            REACTION_TRIGGERS[2],
            REACTION_TRIGGERS[3],
        ];
        let duplicate = [
            LevelReactionImage::new(0, ReactionIndex::new(0)),
            LevelReactionImage::new(0, ReactionIndex::new(0)),
            REACTION_TRIGGERS[2],
            REACTION_TRIGGERS[3],
        ];
        let cases = [
            (
                "unsorted",
                &unsorted[..],
                ImageValidationError::EntriesNotSorted {
                    table: "reaction_triggers",
                    index: 1,
                },
            ),
            (
                "duplicate",
                &duplicate[..],
                ImageValidationError::DuplicateEntry {
                    table: "reaction_triggers",
                    index: 1,
                },
            ),
        ];

        for (name, triggers, expected) in cases {
            let image = EnclaveImage {
                reaction_triggers: triggers,
                ..IMAGE
            };
            assert_eq!(
                EnclaveImageView::new(&image).unwrap_err(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn invalid_ownership_identity_and_storage_report_specific_errors() {
        let bad_modes = [ModeImage::new(ReactorIndex::new(1), ScopeIndex::new(1))];
        let invalid_routes = [RouteImage::new(
            IdentityRange::new(13, 9),
            PortIndex::new(1),
            RouteDirection::Inbound,
            TimingDomain::Logical,
            0,
        )];
        let invalid_identity_data = "plant/control boundary";
        let unsorted_routes = [
            RouteImage::new(
                IdentityRange::new(13, 1),
                PortIndex::new(0),
                RouteDirection::Outbound,
                TimingDomain::Logical,
                0,
            ),
            RouteImage::new(
                IdentityRange::new(14, 1),
                PortIndex::new(1),
                RouteDirection::Inbound,
                TimingDomain::Logical,
                0,
            ),
        ];
        let unsorted_identity_data = "plant/controlza";
        let duplicate_bindings = [
            RequiredBindingImage::new(IdentityRange::new(13, 4), BindingKind::Reaction),
            RequiredBindingImage::new(IdentityRange::new(13, 4), BindingKind::Reaction),
            RequiredBindingImage::new(IdentityRange::new(17, 8), BindingKind::StateInitializer),
            RequiredBindingImage::new(IdentityRange::new(25, 8), BindingKind::StateInitializer),
        ];
        let duplicate_binding_identity_data = "plant/controlsamestate/r0state/r1";
        let invalid_bank_reactors = [
            ReactorImage::new(
                BindingSlotIndex::new(2),
                StateSlotIndex::new(0),
                ScopeIndex::new(0),
                TableRange::new(0, 1),
                Some(ModeIndex::new(0)),
                Some(BankInfoImage::new(2, 2)),
            ),
            REACTORS[1],
        ];
        let cases = [
            (
                "ownership",
                EnclaveImage {
                    modes: &bad_modes,
                    ..IMAGE
                },
                ImageValidationError::OwnershipMismatch {
                    table: "modes",
                    index: 0,
                    field: "scope.reactor",
                },
            ),
            (
                "invalid boundary",
                EnclaveImage {
                    identity_data: invalid_identity_data,
                    routes: &invalid_routes,
                    ..IMAGE
                },
                ImageValidationError::InvalidStableId {
                    kind: "boundary",
                    index: 0,
                    id: " boundary",
                },
            ),
            (
                "unsorted boundary",
                EnclaveImage {
                    identity_data: unsorted_identity_data,
                    routes: &unsorted_routes,
                    ..IMAGE
                },
                ImageValidationError::StableIdsNotSorted {
                    kind: "boundary",
                    index: 1,
                    id: "a",
                },
            ),
            (
                "duplicate binding",
                EnclaveImage {
                    identity_data: duplicate_binding_identity_data,
                    required_bindings: &duplicate_bindings,
                    ..IMAGE
                },
                ImageValidationError::DuplicateStableId {
                    kind: "binding",
                    index: 1,
                    id: "same",
                },
            ),
            (
                "invalid bank",
                EnclaveImage {
                    reactors: &invalid_bank_reactors,
                    ..IMAGE
                },
                ImageValidationError::InvalidBankInfo {
                    reactor: 0,
                    index: 2,
                    total: 2,
                },
            ),
            (
                "state storage bound",
                EnclaveImage {
                    storage_bounds: StorageBounds::new(1, 1, 8, 4),
                    ..IMAGE
                },
                ImageValidationError::StorageBoundExceeded {
                    table: "reactors",
                    index: 1,
                    kind: "state slots",
                    slot: 1,
                    bound: 1,
                },
            ),
        ];

        for (name, image, expected) in cases {
            assert_eq!(
                EnclaveImageView::new(&image).unwrap_err(),
                expected,
                "{name}"
            );
        }
    }
}
