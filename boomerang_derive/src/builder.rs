//! Analyze the data supplied by a ReactorReceiver struct and generate the impl code
//! for the Reactor.

use crate::parse::{PortAttr, ReactionAttr, ReactorReceiver, TimerAttr};

use std::{
    collections::{HashMap, HashSet},
    convert::{TryFrom},
    iter::FromIterator,
    rc::Rc,
};

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct PortBuilder {
    attr: PortAttr,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct TimerBuilder {
    attr: TimerAttr,
}

#[derive(Debug)]
pub struct ReactionBuilder {
    attr: ReactionAttr,
    depends_on_timers: HashSet<Rc<TimerBuilder>>,
    depends_on_inputs: HashSet<Rc<PortBuilder>>,
}

#[derive(Debug, Default)]
pub struct ReactorBuilder {
    idents: HashSet<syn::Ident>,
    timers: HashMap<syn::Ident, Rc<TimerBuilder>>,
    inputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    outputs: HashMap<syn::Ident, Rc<PortBuilder>>,
}

/// Extract the expected (base, member) ident tuple, or None if the ExprField doesn't match.
fn expr_field_parts(expr: &syn::ExprField) -> Option<(&syn::Ident, &syn::Ident)> {
    match *expr.base {
        syn::Expr::Path(syn::ExprPath { ref path, .. }) => path.get_ident(),
        _ => None,
    }
    .and_then(|base| match &expr.member {
        syn::Member::Named(member) => Some((base, member)),
        _ => None,
    })
}

/// Build a map of TimerBuilders from an iterable of TimerAttr
fn build_timers<I, M>(idents: &mut HashSet<syn::Ident>, timers: I) -> Result<M, darling::Error>
where
    I: IntoIterator<Item = TimerAttr>,
    M: FromIterator<(syn::Ident, Rc<TimerBuilder>)>,
{
    timers
        .into_iter()
        .map(|attr| {
            if idents.contains(&attr.name) {
                Err(darling::Error::duplicate_field(&attr.name.to_string()))
            } else {
                idents.insert(attr.name.clone());
                Ok((attr.name.clone(), Rc::new(TimerBuilder { attr: attr })))
            }
        })
        .collect::<Result<M, _>>()
}

fn build_ports<I, M>(idents: &mut HashSet<syn::Ident>, ports: I) -> Result<M, darling::Error>
where
    I: IntoIterator<Item = PortAttr>,
    M: FromIterator<(syn::Ident, Rc<PortBuilder>)>,
{
    ports
        .into_iter()
        .map(|attr| {
            if idents.contains(&attr.name) {
                Err(darling::Error::duplicate_field(&attr.name.to_string()))
            } else {
                idents.insert(attr.name.clone());
                Ok((attr.name.clone(), Rc::new(PortBuilder { attr: attr })))
            }
        })
        .collect::<Result<M, _>>()
}

impl TryFrom<ReactorReceiver> for ReactorBuilder {
    type Error = darling::Error;
    fn try_from(receiver: ReactorReceiver) -> Result<Self, Self::Error> {
        let mut idents = HashSet::<syn::Ident>::new();

        let timers: HashMap<_, _> = build_timers(&mut idents, receiver.timers)?;
        let inputs: HashMap<_, _> = build_ports(&mut idents, receiver.inputs)?;
        let outputs: HashMap<_, _> = build_ports(&mut idents, receiver.outputs)?;

        // Children
        for child in receiver.children.into_iter() {}

        // Reactions
        for reaction in receiver.reactions.into_iter() {
            let mut depends_on_timers = HashSet::new();
            let mut depends_on_inputs = HashSet::new();

            // Triggers can be timers, inputs, outputs of child reactors, or actions
            for trigger in reaction.triggers.iter() {
                let (base, member) = expr_field_parts(trigger)
                    .ok_or(darling::Error::custom("Unexpected expression for trigger"))?;

                if base.to_string() == "self" {
                    if let Some(timer) = timers.get(member) {
                        depends_on_timers.insert(timer.clone());
                    } else if let Some(input) = inputs.get(member) {
                        depends_on_inputs.insert(input.clone());
                    } else {
                        return Err(darling::Error::unknown_field(&format!(
                            "{}.{}",
                            base, member
                        )));
                    }
                }

                // timers.get(trigger).and_then(|timer| {
                // depends_on_timers.insert(timer.clone())
                // });
            }
        }

        Ok(ReactorBuilder {
            idents,
            timers,
            inputs,
            outputs,
        })
    }
}
