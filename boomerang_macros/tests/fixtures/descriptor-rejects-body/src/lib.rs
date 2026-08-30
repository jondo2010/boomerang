use boomerang::prelude::*;

#[reactor(contract = "example.invalid", contract_version = 1)]
pub fn Invalid() -> impl Reactor {
    ctx.add_reaction(Some("builder"))
        .with_reaction_fn(|_, _, ()| {})
        .finish()?;
}
