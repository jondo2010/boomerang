use quote::quote;
use quote::{ToTokens, TokenStreamExt};
use syn::spanned::Spanned;
use syn::{
    parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token, Expr, ExprField, Ident, Token,
};

mod kw {
    syn::custom_keyword!(startup);
    syn::custom_keyword!(shutdown);
}

/// Represents a path or identifier that can be either simple (x) or compound (a.b)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathOrIdent {
    /// A simple identifier
    Simple(Ident),
    /// A field access expression (a.b)
    Field(ExprField),
}

impl Parse for PathOrIdent {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // First try to parse a simple identifier
        if input.peek(Ident) {
            let ident: Ident = input.parse()?;

            // If there's a dot following the identifier, it's a field access
            if input.peek(Token![.]) {
                // Consume the dot
                let dot = input.parse::<Token![.]>()?;

                // Parse the field name
                let member: Ident = input.parse()?;

                // Construct the field expression
                let base = Box::new(Expr::Path(syn::ExprPath {
                    attrs: vec![],
                    qself: None,
                    path: syn::Path {
                        leading_colon: None,
                        segments: [syn::PathSegment {
                            ident,
                            arguments: syn::PathArguments::None,
                        }]
                        .into_iter()
                        .collect(),
                    },
                }));

                return Ok(PathOrIdent::Field(ExprField {
                    attrs: vec![],
                    base,
                    dot_token: dot,
                    member: syn::Member::Named(member),
                }));
            }

            // Simple identifier
            return Ok(PathOrIdent::Simple(ident));
        }

        Err(input.error("expected an identifier or field access expression"))
    }
}

impl ToTokens for PathOrIdent {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            PathOrIdent::Simple(ident) => ident.to_tokens(tokens),
            PathOrIdent::Field(field) => field.to_tokens(tokens),
        }
    }
}

/// For field access expressions, convert a.b.c to a_b_c format.
/// Fallback to "field" if not a chain of field accesses.
fn camelcase_field(field: &ExprField) -> Ident {
    fn flatten(expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Field(f) => {
                flatten(&f.base, out);
                match &f.member {
                    syn::Member::Named(ident) => out.push(ident.to_string()),
                    syn::Member::Unnamed(idx) => out.push(idx.index.to_string()),
                }
            }
            Expr::Path(p) if p.path.segments.len() == 1 => {
                out.push(p.path.segments[0].ident.to_string());
            }
            _ => {}
        }
    }
    let mut parts = Vec::new();
    flatten(&Expr::Field(field.clone()), &mut parts);
    if parts.len() > 1 {
        proc_macro2::Ident::new(&parts.join("_"), field.span())
    } else {
        proc_macro2::Ident::new("field", proc_macro2::Span::call_site())
    }
}

/// Represents the different types of triggers
#[derive(Debug, Clone)]
pub enum TriggerType {
    /// Represents a startup trigger
    Startup,
    /// Represents a shutdown trigger
    Shutdown,
    /// Represents a regular identifier or field access trigger
    Regular(PathOrIdent),
}

impl Parse for TriggerType {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(kw::startup) {
            input.parse::<kw::startup>()?;
            Ok(TriggerType::Startup)
        } else if lookahead.peek(kw::shutdown) {
            input.parse::<kw::shutdown>()?;
            Ok(TriggerType::Shutdown)
        } else {
            // Try to parse as PathOrIdent
            let path_or_ident = input.parse::<PathOrIdent>()?;
            Ok(TriggerType::Regular(path_or_ident))
        }
    }
}

impl ToTokens for TriggerType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            TriggerType::Startup => tokens.append_all(quote!(startup)),
            TriggerType::Shutdown => tokens.append_all(quote!(shutdown)),
            TriggerType::Regular(path_or_ident) => path_or_ident.to_tokens(tokens),
        }
    }
}

/// Parse a reaction definition like:
///
/// ```ignore
/// reaction [name] (triggers) [uses] [-> effects] { ... }
/// reaction (t1) u1, u2 -> e1, e2 { ... }
/// ```
#[derive(Debug)]
pub struct Model {
    name: Option<Ident>,
    triggers: Vec<TriggerType>,
    uses: Vec<PathOrIdent>,
    effects: Vec<PathOrIdent>,
    code: syn::Block,
}

impl Parse for Model {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Parse optional name
        let name = if input.peek(token::Paren) {
            None
        } else {
            Some(input.parse()?)
        };

        // Parse triggers in parentheses
        let content;
        let _paren_token = parenthesized!(content in input);
        let triggers = content.parse_terminated(TriggerType::parse, Token![,])?;

        // Parse uses (if any identifiers before -> or {)
        let mut uses: Punctuated<PathOrIdent, token::Comma> = Punctuated::new();
        while !input.peek(Token![->]) && !input.peek(token::Brace) {
            uses.push_value(input.parse()?);
            if !input.peek(Token![,]) {
                break;
            }
            uses.push_punct(input.parse()?);
        }

        // Parse optional effects (outputs)
        let effects = if input.peek(Token![->]) {
            let _arrow = input.parse::<Token![->]>()?;
            // Parse effects (if any identifiers before {)
            let mut effects: Punctuated<PathOrIdent, token::Comma> = Punctuated::new();
            while !input.peek(token::Brace) {
                effects.push_value(input.parse()?);
                if !input.peek(Token![,]) {
                    break;
                }
                effects.push_punct(input.parse()?);
            }
            Some(effects)
        } else {
            None
        };

        // Parse the code block
        let code = input.parse()?;

        Ok(Model {
            name,
            triggers: triggers.into_iter().collect(),
            uses: uses.into_iter().collect(),
            effects: effects.map(|e| e.into_iter().collect()).unwrap_or_default(),
            code,
        })
    }
}

impl ToTokens for Model {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let name = self.name.as_ref().map_or_else(
            || quote::quote! { None },
            |n| {
                let n_str = n.to_string();
                quote! { Some(#n_str) }
            },
        );

        let triggers = &self.triggers;
        let uses = &self.uses;
        let effects = &self.effects;
        let code = &self.code;

        // Start building the reaction
        let mut reaction = quote! {
            #[allow(unused_variables)]
            let _ = builder
                .add_reaction(#name)
        };

        // Add appropriate trigger methods based on trigger type
        for trigger in triggers {
            match trigger {
                TriggerType::Startup => {
                    reaction.append_all(quote! {
                        .with_startup_trigger()
                    });
                }
                TriggerType::Shutdown => {
                    reaction.append_all(quote! {
                        .with_shutdown_trigger()
                    });
                }
                TriggerType::Regular(path_or_ident) => {
                    reaction.append_all(quote! {
                        .with_trigger(#path_or_ident)
                    });
                }
            }
        }

        // Add trigger arguments for the reaction_fn closure
        let trigger_args = triggers.iter().map(|t| match t {
            TriggerType::Startup => quote!(startup),
            TriggerType::Shutdown => quote!(shutdown),
            TriggerType::Regular(path_or_ident) => {
                let id = match path_or_ident {
                    PathOrIdent::Simple(ident) => ident,
                    PathOrIdent::Field(field) => &camelcase_field(field),
                };
                quote!(#id)
            }
        });

        // Add uses arguments for the reaction_fn closure
        let uses_args = uses.iter().map(|u| match u {
            PathOrIdent::Simple(ident) => quote!(#ident),
            PathOrIdent::Field(field) => {
                let id = &camelcase_field(field);
                quote!(#id)
            }
        });

        // Add effects arguments for the reaction_fn closure
        let effects_args = effects.iter().map(|e| match e {
            PathOrIdent::Simple(ident) => quote!(#ident),
            PathOrIdent::Field(field) => {
                let id = &camelcase_field(field);
                quote!(#id)
            }
        });

        // Add uses and effects
        reaction.append_all(quote! {
            #(.with_use(#uses))*
            #(.with_effect(#effects))*
            .with_reaction_fn(move |
                ctx,
                state, (
                    #(mut #trigger_args,)*
                    #(mut #uses_args,)*
                    #(mut #effects_args,)*
                )| #code
            )
            .finish()?;
        });

        tokens.append_all(reaction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_parse_reaction1() {
        let reaction = syn::parse_str::<Model>("(x) { }").unwrap();
        assert!(reaction.name.is_none());
        assert!(
            matches!(&reaction.triggers[0], TriggerType::Regular(PathOrIdent::Simple(ident)) if ident == "x")
        );
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_empty());
    }

    /// Test reaction with uses and effects
    #[test]
    fn test_parse_reaction2() {
        let reaction = syn::parse_str::<Model>("foo (t1) u1, u2 -> e1, e2 { }").unwrap();
        assert_eq!(reaction.name, parse_quote!(foo));
        assert!(
            matches!(&reaction.triggers[0], TriggerType::Regular(PathOrIdent::Simple(ident)) if ident == "t1")
        );
        assert!(matches!(&reaction.uses[0], PathOrIdent::Simple(ident) if ident == "u1"));
        assert!(matches!(&reaction.uses[1], PathOrIdent::Simple(ident) if ident == "u2"));
        assert!(matches!(&reaction.effects[0], PathOrIdent::Simple(ident) if ident == "e1"));
        assert!(matches!(&reaction.effects[1], PathOrIdent::Simple(ident) if ident == "e2"));
    }

    #[test]
    fn test_parse_reaction3() {
        // Test reaction with just uses
        let reaction = syn::parse_str::<Model>("(x) y, z { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 1);
        assert_eq!(reaction.uses.len(), 2);
        assert!(reaction.effects.is_empty());
    }

    #[test]
    fn test_parse_startup_trigger() {
        // Test reaction with startup trigger
        let reaction = syn::parse_str::<Model>("(startup) { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 1);

        // Check the trigger is a Startup type
        assert!(matches!(&reaction.triggers[0], TriggerType::Startup));
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_empty());
    }

    #[test]
    fn test_parse_shutdown_trigger() {
        // Test reaction with shutdown trigger
        let reaction = syn::parse_str::<Model>("(shutdown) { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 1);

        // Check the trigger is a Shutdown type
        assert!(matches!(&reaction.triggers[0], TriggerType::Shutdown));
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_empty());
    }

    #[test]
    fn test_parse_mixed_triggers() {
        // Test reaction with multiple trigger types
        let reaction = syn::parse_str::<Model>("(startup, x, shutdown) { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 3);

        // Check each trigger is of the expected type
        assert!(matches!(&reaction.triggers[0], TriggerType::Startup));
        assert!(matches!(
            &reaction.triggers[1],
            TriggerType::Regular(PathOrIdent::Simple(ident)) if ident == "x"
        ));
        assert!(matches!(&reaction.triggers[2], TriggerType::Shutdown));
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_empty());
    }

    /// Test reaction with field access expressions
    #[test]
    fn test_parse_compound_paths() {
        let reaction =
            syn::parse_str::<Model>("(module.trigger) module.input -> module.output { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 1);

        // Check the trigger is a Regular type with a compound path
        assert!(matches!(
            &reaction.triggers[0],
            TriggerType::Regular(PathOrIdent::Field(field)) if field == &parse_quote!{ module.trigger }
        ));

        // Check the use item
        assert!(matches!(
            &reaction.uses[0],
            PathOrIdent::Field(field) if field == &parse_quote!{ module.input }
        ));

        // Check the effect item
        assert!(matches!(
            &reaction.effects[0],
            PathOrIdent::Field(field) if field == &parse_quote!{ module.output }
        ));
    }

    #[test]
    fn test_mixed_path_types() {
        // Test reaction with both simple identifiers and field access expressions
        let reaction = syn::parse_str::<Model>("(a.b, c) d, e.f -> g, h.i { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 2);
        assert_eq!(reaction.uses.len(), 2);
        assert_eq!(reaction.effects.len(), 2);

        // Check first trigger is a compound path
        match &reaction.triggers[0] {
            TriggerType::Regular(PathOrIdent::Field(_)) => {}
            _ => panic!("Expected Regular trigger with field access for first trigger"),
        }

        // Check second trigger is a simple identifier
        match &reaction.triggers[1] {
            TriggerType::Regular(PathOrIdent::Simple(ident)) => assert_eq!(ident.to_string(), "c"),
            _ => panic!("Expected Regular trigger with simple identifier for second trigger"),
        }

        // First use should be simple identifier "d"
        match &reaction.uses[0] {
            PathOrIdent::Simple(ident) => assert_eq!(ident.to_string(), "d"),
            _ => panic!("Expected simple identifier for first use"),
        }

        // Second use should be field access "e.f"
        match &reaction.uses[1] {
            PathOrIdent::Field(field) => {
                assert_eq!(field.member.to_token_stream().to_string(), "f")
            }
            _ => panic!("Expected field access for second use"),
        }
    }
}
