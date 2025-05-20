use quote::ToTokens;
use syn::{
    parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token, Ident, Token,
};

mod kw {
    syn::custom_keyword!(input);
    syn::custom_keyword!(output);
    syn::custom_keyword!(reaction);
    syn::custom_keyword!(state);
    syn::custom_keyword!(timer);
    syn::custom_keyword!(child);
}

/// Parse a reaction definition like:
///
///     reaction [name] (triggers) [uses] [-> effects] { ... }
///     reaction (t1) u1, u2 -> e1, e2 { ... }
#[derive(Debug)]
pub struct Model {
    name: Option<Ident>,
    triggers: Vec<Ident>,
    uses: Vec<Ident>,
    effects: Vec<Ident>,
    code: syn::Block,
}

impl Parse for Model {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        println!("{input:?}");

        // Parse: reaction
        //let _reaction = input.parse::<kw::reaction>()?;

        // Parse optional name
        let name = if input.peek(token::Paren) {
            None
        } else {
            Some(input.parse()?)
        };

        // Parse triggers in parentheses
        let content;
        let _paren_token = parenthesized!(content in input);
        let triggers = content.parse_terminated(Ident::parse, Token![,])?;

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
        todo!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::format_ident;
    use syn::parse_quote;

    #[test]
    fn test_parse_reaction() {
        // Test basic reaction
        let reaction: Model = parse_quote! {};

        let reaction = syn::parse_str::<Model>("reaction(x) { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers[0], format_ident!("x"));
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_empty());

        // Test reaction with uses and effects
        let reaction = syn::parse_str::<Model>("reaction foo(t1) u1, u2 -> e1, e2 { }").unwrap();
        assert_eq!(reaction.name, parse_quote!(foo));
        assert_eq!(reaction.triggers[0], format_ident!("t1"));
        assert_eq!(reaction.uses[0], format_ident!("u1"));
        assert_eq!(reaction.uses[1], format_ident!("u2"));
        assert_eq!(reaction.effects[0], format_ident!("e1"));
        assert_eq!(reaction.effects[1], format_ident!("e2"));

        // Test reaction with just uses
        let reaction = syn::parse_str::<Model>("reaction(x) y, z { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 1);
        assert_eq!(reaction.uses.len(), 2);
        assert!(reaction.effects.is_empty());
    }
}
