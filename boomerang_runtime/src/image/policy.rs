//! Closed operational policies stored in compiled deployment images.
//!
//! These enums describe Federate lifecycle and boundary behavior independently
//! of the coordination backend that enforces them. Host-side manifest and
//! deployment layers preserve the typed selections until lowering, while
//! generated artifacts consume their explicit compact discriminants. Open
//! implementation choices, such as transport and codec capabilities, remain
//! separately keyed identities rather than variants in these enums.

macro_rules! policy_enum {
    ($name:ident, $doc:literal, {$($(#[$meta:meta])* $variant:ident = $value:literal => $text:literal),+ $(,)?}) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
        #[repr(u8)]
        pub enum $name {
            $($(#[$meta])* $variant = $value),+
        }

        impl $name {
            /// Returns the canonical manifest spelling of this policy.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    };
}

policy_enum!(RecoveryPolicy, "Closed Federate recovery behavior compiled into a deployment image.", {
    /// Isolate the member and apply downstream failure policies.
    FailStop = 0 => "fail-stop",
    /// Restart the selected artifact from its compiled initial image.
    RestartReset = 1 => "restart-reset",
    /// Retain local state across transport loss and rejoin with a new incarnation.
    TransientRejoin = 2 => "transient-rejoin",
    /// Activate a predefined hot or warm standby.
    RedundantFailover = 3 => "redundant-failover",
    /// Transfer explicitly declared bounded semantic state.
    ApplicationStateTransfer = 4 => "application-state-transfer",
    /// Restore an optional hosted process checkpoint.
    CheckpointRestore = 5 => "checkpoint-restore",
});

policy_enum!(BoundaryFailurePolicy, "Boundary behavior when its source Federate is lost.", {
    /// Stop affected downstream execution.
    PropagateStop = 0 => "propagate-stop",
    /// Produce explicit absence.
    ProduceAbsence = 1 => "produce-absence",
    /// Produce a declared bounded safe value.
    BoundedSafeValue = 2 => "bounded-safe-value",
    /// Enter a declared degraded mode.
    EnterDegradedMode = 3 => "enter-degraded-mode",
    /// Switch to a predefined standby.
    SwitchToStandby = 4 => "switch-to-standby",
});

policy_enum!(TransportPolicy, "Closed transport contract compiled into a boundary image.", {
    /// Reliable, ordered, framed delivery within one membership epoch.
    ReliableOrderedFramed = 0 => "reliable-ordered-framed",
});

policy_enum!(CodecPolicy, "Closed codec contract compiled into a boundary image.", {
    /// Canonical architecture-independent encoding into bounded storage.
    CanonicalBounded = 0 => "canonical-bounded",
});

policy_enum!(TimingPolicy, "Closed physical-time contract compiled into a boundary image.", {
    /// Worst-case contract requiring complete qualification evidence.
    HardBound = 0 => "hard-bound",
    /// Target-window objective with explicit miss behavior.
    SoftTarget = 1 => "soft-target",
    /// Bounded resource use without a response-time guarantee.
    BestEffort = 2 => "best-effort",
});

policy_enum!(SecurityPolicy, "Closed communication security profile compiled into a boundary image.", {
    /// No channel security, accepted only by the deployment threat model.
    None = 0 => "none",
    /// Integrity protection on an otherwise protected link.
    IntegrityOnly = 1 => "integrity-only",
    /// Federate authentication without payload confidentiality.
    Authenticated = 2 => "authenticated",
    /// Federate authentication, integrity, and encryption.
    AuthenticatedEncrypted = 3 => "authenticated-encrypted",
});
