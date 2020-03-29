//! Analyze the data supplied by a ReactorReceiver struct and generate the impl code
//! for the Reactor.

use crate::parse::{PortAttr, ReactionAttr, ReactorReceiver, TimerAttr};

use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
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
    depends_on_timers: Vec<Rc<TimerBuilder>>,
    depends_on_inputs: Vec<Rc<PortBuilder>>,
}

#[derive(Debug, Default)]
pub struct ReactorBuilder {
    idents: HashSet<syn::Ident>,
    timers: HashMap<syn::Ident, Rc<TimerBuilder>>,
    inputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    outputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    reactions: Vec<ReactionBuilder>,
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
        for child in receiver.children.into_iter() {
            todo!();
        }

        // Reactions
        let reactions = receiver
            .reactions
            .into_iter()
            .map(|reaction| {
                // Triggers can be timers, inputs, outputs of child reactors, or actions
                let triggers = reaction
                    .triggers
                    .iter()
                    .map(|trigger| {
                        expr_field_parts(trigger)
                            .ok_or(darling::Error::custom("Unexpected expression for trigger"))
                            .map(|(base, member)| {
                                let trigger_name = format!("{}.{}", base, member);
                                if base.to_string() == "self" {
                                    (
                                        trigger_name,
                                        timers.get(member).cloned(),
                                        inputs.get(member).cloned(),
                                    )
                                } else {
                                    todo!("Outputs of child reactors");
                                    (trigger_name, None, None)
                                }
                            })
                            .and_then(|(field_name, timer, input)| match (&timer, &input) {
                                (None, None) => Err(darling::Error::unknown_field(&field_name)),
                                _ => Ok((timer, input)),
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let (timers, inputs): (Vec<_>, Vec<_>) = triggers.into_iter().unzip();

                Ok(ReactionBuilder {
                    attr: reaction,
                    depends_on_timers: timers.into_iter().filter_map(|x| x).collect(),
                    depends_on_inputs: inputs.into_iter().filter_map(|x| x).collect(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ReactorBuilder {
            idents,
            timers,
            inputs,
            outputs,
            reactions,
        })
    }
}
