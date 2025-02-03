use syn::{
    braced, parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token, Error, Expr, Ident, Token, Type,
};

mod kw {
    syn::custom_keyword!(input);
    syn::custom_keyword!(output);
    syn::custom_keyword!(reaction);
    syn::custom_keyword!(state);
    syn::custom_keyword!(timer);
    syn::custom_keyword!(child);
}

mod render {
    use quote::{format_ident, quote, ToTokens, TokenStreamExt};
    use syn::LitStr;

    use super::Reactor;

    impl ToTokens for Reactor {
        fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
            let Self {
                ident,
                params,
                block,
                ..
            } = self;

            let builder_name_doc = LitStr::new(
                &format!(" Props for the [`{ident}`] component."),
                ident.span(),
            );

            let props_name = format_ident!("{ident}Props");
            let impl_generics = quote! {};
            let where_clause = quote! {};
            let prop_builder_fields = quote! {};

            let output = quote! {
                #[doc = #builder_name_doc]
                #[doc = ""]
                //#docs_and_prop_docs
                #[derive(::boomerang::typed_builder_macro::TypedBuilder)]
                #[builder(crate_module_path = ::boomerang::typed_builder)]
                #[allow(non_snake_case)]
                pub struct #props_name #impl_generics #where_clause {
                    #prop_builder_fields
                }
            };

            tokens.append_all(output);
        }
    }
}

/// Parse a reactor definition like:
///
///     Scale(scale: u32 = 2) {
///         input x: u32,
///         output y: u32,
///         reaction(x) -> y {}
///     }
#[derive(Debug)]
pub struct Reactor {
    pub ident: Ident,
    paren_token: Option<token::Paren>,
    params: Option<Punctuated<Param, Token![,]>>,
    block: ReactorBlock,
}

impl Parse for Reactor {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = input.parse()?;

        let (paren_token, params) = if input.peek(token::Paren) {
            let args_content;
            let paren = parenthesized!(args_content in input);
            let params = args_content.parse_terminated(Param::parse, Token![,])?;
            (Some(paren), Some(params))
        } else {
            (None, None)
        };

        Ok(Reactor {
            ident,
            paren_token,
            params,
            block: input.parse()?,
        })
    }
}

/// Parse a reactor parameter definition, with an optional default value:
///
///    scale: u32 = 2
///    x: u32
#[derive(Debug, PartialEq)]
struct Param {
    name: Ident,
    colon_token: Token![:],
    ty: Type,
    eq_token: Option<Token![=]>,
    default: Option<syn::Expr>,
}

impl Parse for Param {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = input.parse()?;
        let colon_token = input.parse()?;
        let ty = input.parse()?;
        let eq_token: Option<Token![=]> = input.parse()?;
        let default = if eq_token.is_some() {
            Some(input.parse()?)
        } else {
            None
        };

        Ok(Self {
            name,
            colon_token,
            ty,
            eq_token,
            default,
        })
    }
}

/// Parse a block of statements within a reactor definition
#[derive(Debug)]
pub struct ReactorBlock {
    pub brace_token: token::Brace,
    /// Statements in a block
    pub stmts: Vec<ReactorStmt>,
}

impl ReactorBlock {
    pub fn parse_within(input: ParseStream) -> Result<Vec<ReactorStmt>, Error> {
        let mut stmts = Vec::new();
        loop {
            if input.is_empty() {
                break;
            }
            let stmt = parse_stmt(input)?;
            let requires_semicolon = match &stmt {
                ReactorStmt::Reaction(..) => false,
                _ => true,
            };
            stmts.push(stmt);
            if input.is_empty() {
                break;
            } else if requires_semicolon {
                input.parse::<Token![;]>()?;
            }
        }
        Ok(stmts)
    }
}

impl Parse for ReactorBlock {
    fn parse(input: ParseStream) -> Result<Self, Error> {
        let content;
        Ok(ReactorBlock {
            brace_token: braced!(content in input),
            stmts: content.call(Self::parse_within)?,
        })
    }
}

#[derive(Debug)]
enum ReactorStmt {
    Input(Input),
    Output(Output),
    Timer(Timer),
    State(State),
    Connection(Connection),
    Child(Child),
    Reaction(Reaction),
}

#[derive(Debug)]
struct Input {
    input: kw::input,
    name: Ident,
    colon_token: Token![:],
    ty: Type,
}

impl Parse for Input {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Input {
            input: input.parse()?,
            name: input.parse()?,
            colon_token: input.parse()?,
            ty: input.parse()?,
        })
    }
}

#[derive(Debug)]
struct Output {
    output: kw::output,
    name: Ident,
    colon_token: Token![:],
    ty: Type,
}

impl Parse for Output {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Output {
            output: input.parse()?,
            name: input.parse()?,
            colon_token: input.parse()?,
            ty: input.parse()?,
        })
    }
}

#[derive(Debug)]
struct TimerSpec {
    paren_token: token::Paren,
    offset: syn::Expr,
    comma: Token![,],
    period: syn::Expr,
}

impl Parse for TimerSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        let paren_token = parenthesized!(content in input);
        dbg!(&content);
        let offset = content.parse()?;
        let comma = content.parse()?;
        let period = content.parse()?;
        Ok(TimerSpec {
            paren_token,
            offset,
            comma,
            period,
        })
    }
}

/// Parse a timer definition like:
///
///    timer tim;
///    timer t_spec(0, 1 sec);
#[derive(Debug)]
struct Timer {
    timer: kw::timer,
    name: Ident,
    paren_token: Option<token::Paren>,
    spec: Option<Punctuated<Expr, Token![,]>>,
}

impl Parse for Timer {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let timer = input.parse()?;
        let name = input.parse()?;
        let (paren_token, spec) = if input.peek(token::Paren) {
            let spec_content;
            let paren_token = parenthesized!(spec_content in input);
            let spec = spec_content.parse_terminated(Expr::parse, Token![,])?;
            (Some(paren_token), Some(spec))
        } else {
            (None, None)
        };
        Ok(Timer {
            timer,
            name,
            paren_token,
            spec,
        })
    }
}

#[derive(Debug)]
struct State {
    state: kw::state,
    name: Ident,
    colon_token: Token![:],
    ty: Type,
    eq_token: Option<Token![=]>,
    default: Option<syn::Expr>,
}

impl Parse for State {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(State {
            state: input.parse()?,
            name: input.parse()?,
            colon_token: input.parse()?,
            ty: input.parse()?,
            eq_token: input.parse()?,
            default: if input.peek(Token![=]) {
                Some(input.parse()?)
            } else {
                None
            },
        })
    }
}

#[derive(Debug)]
struct Child {
    child: kw::child,
    name: Ident,
    colon_token: Token![:],
    ty: Ident,
    args: Option<(token::Paren, Punctuated<ChildParam, Token![,]>)>,
}

impl Parse for Child {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        Ok(Child {
            child: input.parse()?,
            name: input.parse()?,
            colon_token: input.parse()?,
            ty: input.parse()?,
            args: if input.peek(token::Paren) {
                Some((
                    parenthesized!(content in input),
                    content.parse_terminated(ChildParam::parse, Token![,])?,
                ))
            } else {
                None
            },
        })
    }
}

/// Parse a child parameter definition like:
///    scale = 2
#[derive(Debug)]
pub struct ChildParam {
    pub ident: Ident,
    pub eq_token: Token![=],
    pub value: Expr,
}

impl Parse for ChildParam {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(ChildParam {
            ident: input.parse()?,
            eq_token: input.parse()?,
            value: input.parse()?,
        })
    }
}

#[derive(Debug)]
struct Connection {
    from_reactor: Option<Ident>,
    from_port: Ident,
    arrow: Token![->],
    to_reactor: Option<Ident>,
    to_port: Ident,
}

impl Parse for Connection {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let from = input.parse::<Ident>()?;
        let (from_reactor, from_port) = if input.peek(Token![.]) {
            let _dot: Token![.] = input.parse()?;
            (Some(from), input.parse()?)
        } else {
            (None, from)
        };

        let arrow = input.parse()?;

        let to = input.parse::<Ident>()?;
        let (to_reactor, to_port) = if input.peek(Token![.]) {
            let _dot: Token![.] = input.parse()?;
            (Some(to), input.parse()?)
        } else {
            (None, to)
        };

        Ok(Connection {
            from_reactor,
            from_port,
            arrow,
            to_reactor,
            to_port,
        })
    }
}

/// Parse a reaction definition like:
///
///     reaction [name] (triggers) [uses] [-> effects] { ... }
///     reaction (t1) u1, u2 -> e1, e2 { ... }
#[derive(Debug)]
struct Reaction {
    reaction: kw::reaction,
    name: Option<Ident>,
    paren_token: token::Paren,
    triggers: Punctuated<Ident, Token![,]>,
    uses: Punctuated<Ident, Token![,]>,
    effects: Option<(Token![->], Punctuated<Ident, Token![,]>)>,
    code: syn::Block,
}

impl Parse for Reaction {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Parse: reaction
        let reaction = input.parse::<kw::reaction>()?;

        // Parse optional name
        let name = if input.peek(token::Paren) {
            None
        } else {
            Some(input.parse()?)
        };

        // Parse triggers in parentheses
        let content;
        let paren_token = parenthesized!(content in input);
        let triggers = content.parse_terminated(Ident::parse, Token![,])?;

        // Parse uses (if any identifiers before -> or {)
        let mut uses = Punctuated::new();
        while !input.peek(Token![->]) && !input.peek(token::Brace) {
            uses.push_value(input.parse()?);
            if !input.peek(Token![,]) {
                break;
            }
            uses.push_punct(input.parse()?);
        }

        // Parse optional effects (outputs)
        let effects = if input.peek(Token![->]) {
            let arrow = input.parse()?;
            // Parse effects (if any identifiers before {)
            let mut effects = Punctuated::new();
            while !input.peek(token::Brace) {
                effects.push_value(input.parse()?);
                if !input.peek(Token![,]) {
                    break;
                }
                effects.push_punct(input.parse()?);
            }
            Some((arrow, effects))
        } else {
            None
        };

        // Parse the code block
        let code = input.parse()?;

        Ok(Reaction {
            reaction,
            name,
            paren_token,
            triggers,
            uses,
            effects,
            code,
        })
    }
}

fn parse_stmt(input: ParseStream) -> Result<ReactorStmt, Error> {
    let lookahead = input.lookahead1();

    Ok(if lookahead.peek(kw::input) {
        ReactorStmt::Input(input.parse()?)
    } else if lookahead.peek(kw::output) {
        ReactorStmt::Output(input.parse()?)
    } else if lookahead.peek(kw::timer) {
        ReactorStmt::Timer(input.parse()?)
    } else if lookahead.peek(kw::state) {
        ReactorStmt::State(input.parse()?)
    } else if lookahead.peek(kw::child) {
        ReactorStmt::Child(input.parse()?)
    } else if lookahead.peek(kw::reaction) {
        ReactorStmt::Reaction(input.parse()?)
    } else if lookahead.peek(Ident) {
        ReactorStmt::Connection(input.parse()?)
    } else {
        return Err(lookahead.error());
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use quote::format_ident;
    use syn::parse_quote;

    #[test]
    fn test_parse_input() {
        let input = syn::parse_str::<Input>("input x: u32").unwrap();
        assert_eq!(input.name, "x");
        assert_eq!(input.ty, parse_quote! {u32});
    }

    #[test]
    fn test_parse_output() {
        let input = syn::parse_str::<Output>("output y: u32").unwrap();
        assert_eq!(input.name, "y");
        assert_eq!(input.ty, parse_quote! {u32});
    }

    #[test]
    fn test_parse_timer() {
        // Test simple timer
        let timer = syn::parse_str::<Timer>("timer tim").unwrap();
        assert_eq!(timer.name.to_string(), "tim");
        assert!(timer.spec.is_none());

        // Test timer with specification
        let timer = syn::parse_str::<Timer>("timer t(0, 1)").unwrap();
        assert_eq!(timer.name.to_string(), "t");
        //assert!(timer.spec.is_some());
        //let spec = timer.spec.as_ref().unwrap();
        //assert_eq!(spec.offset, parse_quote!(0));
        //assert_eq!(spec.period, parse_quote!(1 sec));
    }

    #[test]
    fn test_parse_child() {
        let child = syn::parse_str::<Child>("child g: Scale()").unwrap();
        assert_eq!(child.name, format_ident!("g"));

        let child = syn::parse_str::<Child>("child g: Scale(scale = 2)").unwrap();
        assert_eq!(child.name, format_ident!("g"));
    }

    #[test]
    fn test_parse_reaction() {
        // Test basic reaction
        let reaction = syn::parse_str::<Reaction>("reaction(x) { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers, parse_quote!(x));
        assert!(reaction.uses.is_empty());
        assert!(reaction.effects.is_none());

        // Test reaction with uses and effects
        let reaction = syn::parse_str::<Reaction>("reaction foo(t1) u1, u2 -> e1, e2 { }").unwrap();
        assert_eq!(reaction.name, parse_quote!(foo));
        assert_eq!(reaction.triggers, parse_quote!(t1));
        assert_eq!(reaction.uses, parse_quote!(u1, u2));
        assert_eq!(reaction.effects.as_ref().unwrap().1, parse_quote!(e1, e2));

        // Test reaction with just uses
        let reaction = syn::parse_str::<Reaction>("reaction(x) y, z { }").unwrap();
        assert!(reaction.name.is_none());
        assert_eq!(reaction.triggers.len(), 1);
        assert_eq!(reaction.uses.len(), 2);
        assert!(reaction.effects.is_none());
    }

    #[test]
    fn test_parse_reactor() {
        let reactor = syn::parse_str::<Reactor>("Test {}").unwrap();
        assert_eq!(reactor.ident, format_ident!("Test"));
        assert!(reactor.params.is_none());

        let reactor = syn::parse_str::<Reactor>("Scale() {}").unwrap();
        assert_eq!(reactor.ident, format_ident!("Scale"));
        assert!(reactor.params.unwrap().is_empty());

        let reactor = syn::parse_str::<Reactor>("Scale(scale: u32 = 2) {}").unwrap();
        assert_eq!(reactor.ident, format_ident!("Scale"));
        assert_eq!(reactor.params.unwrap(), parse_quote!(scale: u32 = 2));

        let reactor = syn::parse_str::<Reactor>("Scale() { input x: u32; }").unwrap();
        assert_eq!(reactor.ident, format_ident!("Scale"));
        assert!(reactor.params.unwrap().is_empty());

        let reactor = syn::parse_str::<Reactor>(
            r#"Scale(scale: u32 = 2) {
                input x: u32;
                output y: u32;
                reaction(x) -> y {}
            }"#,
        )
        .unwrap();
        assert_eq!(reactor.ident, format_ident!("Scale"));
        assert_eq!(reactor.params.unwrap(), parse_quote!(scale: u32 = 2));
        assert_eq!(reactor.block.stmts.len(), 3);
    }
}
