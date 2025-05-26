use quote::quote;
use quote::{ToTokens, TokenStreamExt};
use syn::{
    parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token, Ident, Token,
};

mod kw {
    syn::custom_keyword!(startup);
    syn::custom_keyword!(shutdown);
}

/// Represents the different types of triggers
#[derive(Debug, Clone)]
pub enum TriggerType {
    /// Represents a startup trigger
    Startup,
    /// Represents a shutdown trigger
    Shutdown,
    /// Represents a regular identifier trigger
    Regular(Ident),
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
        } else if lookahead.peek(Ident) {
            Ok(TriggerType::Regular(input.parse()?))
        } else {
            Err(lookahead.error())
        }
    }
}

impl ToTokens for TriggerType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            TriggerType::Startup => tokens.append_all(quote!(startup)),
            TriggerType::Shutdown => tokens.append_all(quote!(shutdown)),
            TriggerType::Regular(ident) => ident.to_tokens(tokens),
        }
    }
}

/// Parse a reaction definition like:
///
///     reaction [name] (triggers) [uses] [-> effects] { ... }
///     reaction (t1) u1, u2 -> e1, e2 { ... }
#[derive(Debug)]
pub struct Model {
    name: Option<Ident>,
    triggers: Vec<TriggerType>,
    uses: Vec<Ident>,
    effects: Vec<Ident>,
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
        let mut uses: Punctuated<Ident, token::Comma> = Punctuated::new();
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
            let mut effects: Punctuated<Ident, token::Comma> = Punctuated::new();
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
                .add_reaction2(#name)
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
                TriggerType::Regular(ident) => {
                    reaction.append_all(quote! {
                        .with_trigger(#ident)
                    });
                }
            }
        }

        // Add trigger arguments for the reaction_fn closure
        let trigger_args = triggers.iter().map(|t| match t {
            TriggerType::Startup => quote!(startup),
            TriggerType::Shutdown => quote!(shutdown),
            TriggerType::Regular(ident) => quote!(#ident),
        });

        // Add uses and effects
        reaction.append_all(quote! {
            #(.with_use(#uses))*
            #(.with_effect(#effects))*
            .with_reaction_fn(|
                ctx,
                state, (
                    #(mut #trigger_args,)*
                    #(mut #uses,)*
                    #(mut #effects,)*
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
    use quote::format_ident;
    use syn::parse_quote;

    #[test]
    fn test_parse_reaction1() {
        let reaction = syn::parse_str::<Model>("(x) { }").unwrap();
        assert!(reaction.name.is_none());
        assert!(matches!(&reaction.triggers[0], TriggerType::Regular(ident) if ident == "x"));
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_empty());
    }

    #[test]
    fn test_parse_reaction2() {
        // Test reaction with uses and effects
        let reaction = syn::parse_str::<Model>("foo (t1) u1, u2 -> e1, e2 { }").unwrap();
        assert_eq!(reaction.name, parse_quote!(foo));

        // Check the trigger is a Regular type with value "t1"
        match &reaction.triggers[0] {
            TriggerType::Regular(ident) => assert_eq!(ident.to_string(), "t1"),
            _ => panic!("Expected Regular trigger but got something else"),
        }

        assert_eq!(reaction.uses[0], format_ident!("u1"));
        assert_eq!(reaction.uses[1], format_ident!("u2"));
        assert_eq!(reaction.effects[0], format_ident!("e1"));
        assert_eq!(reaction.effects[1], format_ident!("e2"));
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
            TriggerType::Regular(ident) if ident == "x"
        ));
        assert!(matches!(&reaction.triggers[2], TriggerType::Shutdown));
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_empty());
    }
}
