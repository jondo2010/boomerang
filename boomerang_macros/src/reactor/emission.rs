//! Common component selection for helper items and reactor output.
//!
//! Macro entry points supply [`ComponentEmission`] selector inputs. This policy
//! emits caller-owned Cargo feature predicates for rustc to resolve.
//! Explicit metadata and payload inputs select unconditional output.
//!
//! Payload compatibility derives from the component declaration.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

/// Selector input supplied by a component macro or direct reactor attribute.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentEmission {
    /// Emit the hosted API for a direct reactor attribute.
    Hosted,
    /// Resolve additive caller features with hosted fallback.
    Features,
    /// Emit native metadata independently of caller features.
    Metadata,
    /// Emit native payload independently of caller features.
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
    /// Hosted API selection, including the feature-free fallback.
    hosted: Selection,
    /// Metadata selection, independent from additive payload selection.
    metadata: Selection,
    /// Payload selection, independent from additive metadata selection.
    payload: Selection,
}

impl SelectionPolicy {
    /// Convert selector inputs into the only table of caller-owned cfg predicates.
    pub(crate) fn new(input: ComponentEmission) -> Self {
        let (hosted, metadata, payload) = match input {
            ComponentEmission::Hosted => (Selection::Always, Selection::Never, Selection::Never),
            ComponentEmission::Features => (
                Selection::Cfg {
                    predicate: quote!(not(any(feature = "metadata", feature = "payload"))),
                },
                Selection::Cfg {
                    predicate: quote!(feature = "metadata"),
                },
                Selection::Cfg {
                    predicate: quote!(feature = "payload"),
                },
            ),
            ComponentEmission::Metadata => (Selection::Never, Selection::Always, Selection::Never),
            ComponentEmission::Payload => (Selection::Never, Selection::Never, Selection::Always),
        };
        Self {
            hosted,
            metadata,
            payload,
        }
    }

    /// Retain helpers exactly when a selected hosted or payload mode needs them.
    pub(crate) fn helper(&self, item: TokenStream) -> TokenStream {
        self.hosted.or(&self.payload).item(item)
    }

    /// Determine whether any caller configuration selects payload emission.
    pub(super) fn needs_payload(&self) -> bool {
        !matches!(self.payload, Selection::Never)
    }

    /// Apply common selection to output fragments and their public namespaces.
    pub(super) fn output(&self, reactor: &Ident, output: ReactorOutput) -> TokenStream {
        let hosted = self.hosted.output(reactor, "hosted", output.hosted);
        let native_payload = self
            .payload
            .output(reactor, "native_payload", output.native_payload);
        let topology = self
            .metadata
            .output(reactor, "native_metadata", output.topology);
        let metadata_contents = output.native_metadata;
        let payload_exports = output.native_payload_exports;
        let metadata = self.metadata.item(quote! {
            pub mod metadata { #metadata_contents }
        });
        let payload = self.payload.item(quote! {
            pub mod payload {
                #[allow(unused_imports)]
                use super::super::*;
                #payload_exports
            }
        });
        let namespace = self.metadata.or(&self.payload).output(
            reactor,
            "native_namespace",
            quote! {
                pub mod __boomerang { #metadata #payload }
            },
        );
        quote! {
            #hosted #native_payload #namespace #topology
        }
    }
}
