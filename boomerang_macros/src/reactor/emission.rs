//! Common component selection for helper items and reactor output.
//!
//! Macro entry points supply [`ComponentEmission`] selector inputs. This policy
//! emits caller-owned feature and wrapper cfg predicates for rustc to resolve.
//! Explicit metadata and payload inputs select unconditional output.
//!
//! Selection does not define compatibility protocols. The shared payload generator
//! retains native self-derived compatibility and both legacy payload protocols.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

/// Selector input supplied by a component macro or direct reactor attribute.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentEmission {
    /// Preserve direct reactor selection through wrapper-injected cfg values.
    Legacy,
    /// Resolve additive caller features with legacy wrapper fallback.
    Features,
    /// Emit native metadata independently of caller features and wrapper cfg.
    Metadata,
    /// Emit native payload independently of caller features and wrapper cfg.
    Payload,
}

/// One selected output, resolved immediately or by the invoking crate's rustc.
#[derive(Clone)]
enum Selection {
    /// Emit the output without a cfg predicate.
    Always,
    /// Omit the output without emitting its tokens.
    Never,
    /// Let rustc resolve the invoking crate's selection.
    Cfg {
        /// Predicate evaluated in the invoking crate, rather than the proc-macro crate.
        predicate: TokenStream,
    },
}

impl Selection {
    fn and(&self, predicate: TokenStream) -> Self {
        match self {
            Self::Always => Self::Cfg { predicate },
            Self::Never => Self::Never,
            Self::Cfg { predicate: left } => Self::Cfg {
                predicate: quote!(all(#left, #predicate)),
            },
        }
    }

    fn or(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Always, _) | (_, Self::Always) => Self::Always,
            (Self::Never, right) => right.clone(),
            (left, Self::Never) => left.clone(),
            (Self::Cfg { predicate: left }, Self::Cfg { predicate: right }) => Self::Cfg {
                predicate: quote!(any(#left, #right)),
            },
        }
    }

    /// Select one item before rustc expands its attributes or resolves its names.
    fn item(&self, item: TokenStream) -> TokenStream {
        match self {
            Self::Always => item,
            Self::Never => TokenStream::new(),
            Self::Cfg { predicate } => quote!(#[cfg(#predicate)] #item),
        }
    }

    /// Delay expansion of an output group until rustc resolves caller-owned cfg.
    /// Macro suppression also removes implementation-only attributes and names.
    fn output(&self, reactor: &Ident, suffix: &str, output: TokenStream) -> TokenStream {
        match self {
            Self::Always => output,
            Self::Never => TokenStream::new(),
            Self::Cfg { predicate } => {
                let selector = format_ident!("__boomerang_select_{}_{}", reactor, suffix);
                quote! {
                    #[allow(non_snake_case, unexpected_cfgs)]
                    mod #selector {
                        #[cfg(#predicate)]
                        macro_rules! emit { ($($tokens:tt)*) => { $($tokens)* }; }
                        #[cfg(not(#predicate))]
                        macro_rules! emit { ($($tokens:tt)*) => {} }
                        pub(super) use emit;
                    }
                    #selector::emit! { #output }
                }
            }
        }
    }
}

/// Output fragments from the existing parser and shared generators.
pub(super) struct ReactorOutput {
    /// Hosted reactor API, capability declarations, and topology constructors.
    pub hosted: TokenStream,
    /// Legacy descriptor namespace and its single-reactor guard.
    pub legacy_descriptor: TokenStream,
    /// Legacy payload namespace with its existing compatibility protocol.
    pub legacy_payload: TokenStream,
    /// Native descriptor contents for the additive metadata namespace.
    pub native_metadata: TokenStream,
    /// Native typed topology constructors selected with metadata.
    pub topology: TokenStream,
    /// Native payload definitions retained in the component's lexical scope.
    pub native_payload: TokenStream,
    /// Native compatibility manifest and aliases for the payload namespace.
    pub native_payload_exports: TokenStream,
}

/// Common output modes used for reactor output and implementation helpers.
pub(crate) struct SelectionPolicy {
    /// Hosted API selection, including the feature-free legacy fallback.
    hosted: Selection,
    /// Legacy descriptor selection without payload selection.
    legacy_descriptor: Selection,
    /// Legacy payload selection without descriptor selection.
    legacy_payload: Selection,
    /// Native metadata selection, independent from additive payload selection.
    native_metadata: Selection,
    /// Native payload selection, independent from additive metadata selection.
    native_payload: Selection,
    /// Conflicting legacy descriptor and payload selection.
    legacy_conflict: Selection,
    /// Conflicting public features and wrapper-injected facet selection.
    native_conflict: Selection,
}

impl SelectionPolicy {
    /// Convert selector inputs into the only table of caller-owned cfg predicates.
    pub(crate) fn new(input: ComponentEmission) -> Self {
        let descriptor = quote!(boomerang_facet = "descriptor");
        let payload = quote!(boomerang_facet = "payload");
        let metadata_feature = quote!(feature = "metadata");
        let payload_feature = quote!(feature = "payload");
        let no_facets = quote!(not(any(#descriptor, #payload)));
        let any_features = quote!(any(#metadata_feature, #payload_feature));
        let (legacy, native_metadata, native_payload, native_conflict) = match input {
            ComponentEmission::Legacy => (
                Selection::Always,
                Selection::Never,
                Selection::Never,
                Selection::Never,
            ),
            ComponentEmission::Features => (
                Selection::Cfg {
                    predicate: quote!(not(#any_features)),
                },
                Selection::Cfg {
                    predicate: quote!(all(#no_facets, #metadata_feature)),
                },
                Selection::Cfg {
                    predicate: quote!(all(#no_facets, #payload_feature)),
                },
                Selection::Cfg {
                    predicate: quote!(all(not(#no_facets), #any_features)),
                },
            ),
            ComponentEmission::Metadata => (
                Selection::Never,
                Selection::Always,
                Selection::Never,
                Selection::Never,
            ),
            ComponentEmission::Payload => (
                Selection::Never,
                Selection::Never,
                Selection::Always,
                Selection::Never,
            ),
        };
        Self {
            hosted: legacy.and(no_facets),
            legacy_descriptor: legacy.and(quote!(all(#descriptor, not(#payload)))),
            legacy_payload: legacy.and(quote!(all(#payload, not(#descriptor)))),
            legacy_conflict: legacy.and(quote!(all(#descriptor, #payload))),
            native_metadata,
            native_payload,
            native_conflict,
        }
    }

    /// Retain helpers exactly when a selected hosted or payload mode needs them.
    pub(crate) fn helper(&self, item: TokenStream) -> TokenStream {
        self.hosted
            .or(&self.legacy_payload)
            .or(&self.native_payload)
            .item(item)
    }

    /// Determine whether any caller configuration selects the legacy payload generator.
    pub(super) fn needs_legacy_payload(&self) -> bool {
        !matches!(self.legacy_payload, Selection::Never)
    }

    /// Determine whether any caller configuration selects the native payload generator.
    pub(super) fn needs_native_payload(&self) -> bool {
        !matches!(self.native_payload, Selection::Never)
    }

    /// Apply common selection to output fragments and their public namespaces.
    pub(super) fn output(&self, reactor: &Ident, output: ReactorOutput) -> TokenStream {
        let hosted = self.hosted.output(reactor, "hosted", output.hosted);
        let descriptor =
            self.legacy_descriptor
                .output(reactor, "descriptor", output.legacy_descriptor);
        let legacy_payload = self
            .legacy_payload
            .output(reactor, "payload", output.legacy_payload);
        let native_payload =
            self.native_payload
                .output(reactor, "native_payload", output.native_payload);
        let topology = self
            .native_metadata
            .output(reactor, "native_metadata", output.topology);
        let metadata_contents = output.native_metadata;
        let payload_exports = output.native_payload_exports;
        let metadata = self.native_metadata.item(quote! {
            pub mod metadata { #metadata_contents }
        });
        let payload = self.native_payload.item(quote! {
            pub mod payload {
                #[allow(unused_imports)]
                use super::super::*;
                #payload_exports
            }
        });
        let namespace = self.native_metadata.or(&self.native_payload).output(
            reactor,
            "native_namespace",
            quote! {
                pub mod __boomerang { #metadata #payload }
            },
        );
        let legacy_conflict = self.legacy_conflict.output(
            reactor,
            "conflict",
            quote! {
                compile_error!("boomerang_facet cannot select both descriptor and payload");
            },
        );
        let native_conflict = self.native_conflict.output(reactor, "native_conflict", quote! {
            compile_error!("cannot combine boomerang_facet with component metadata/payload features");
        });
        quote! {
            #hosted #descriptor #legacy_payload #native_payload
            #namespace #topology #legacy_conflict #native_conflict
        }
    }
}
