use boomerang_builder::compiler::*;
use std::cell::RefCell;

#[allow(dead_code)]
struct Payload(std::rc::Rc<()>); // Deliberately no runtime payload traits.
struct Definition;
impl ComponentDefinition for Definition {
    type Ports = (TopologyInput<Payload>, TopologyOutput<Payload>);
    fn contract(&self) -> (&str, u64) {
        ("test", 1)
    }
    fn declare(
        &self,
        ctx: &mut ComponentTopology<'_>,
    ) -> Result<Self::Ports, TopologyAuthoringError> {
        Ok((ctx.input("in", 0)?, ctx.output("out", 1)?))
    }
}

#[test]
fn typed_connections_preserve_canonical_ids_and_reject_foreign_handles() {
    let mut app = TopologyBuilder::new("application/a/b").unwrap();
    let enclave = app.enclave("a").unwrap();
    let a = app.component("a", Definition, &enclave).unwrap();
    let b = app.component("b", Definition, &enclave).unwrap();
    app.connect(&a.1, &b.0).unwrap();
    let mut other = TopologyBuilder::new("application/a/b").unwrap();
    assert!(matches!(
        other.component("a", Definition, &enclave),
        Err(TopologyAuthoringError::ForeignHandle)
    ));
    let foreign = other.enclave("a").unwrap();
    let other_a = other.component("a", Definition, &foreign).unwrap();
    assert!(matches!(
        app.connect(&other_a.1, &b.0),
        Err(TopologyAuthoringError::ForeignHandle)
    ));
    assert!(app.component("a", Definition, &enclave).is_err());
    let graph = app.finish().unwrap();
    assert_eq!(graph.components().count(), 2);
    assert_eq!(graph.actions().count(), 4);
    assert_eq!(
        graph.connections().next().unwrap().0.to_string(),
        "boundary/a%2Fout/b%2Fin/c0"
    );
}

struct Failed<'a>(&'a RefCell<Option<TopologyOutput<Payload>>>);
impl ComponentDefinition for Failed<'_> {
    type Ports = ();
    fn contract(&self) -> (&str, u64) {
        ("test", 1)
    }
    fn declare(&self, ctx: &mut ComponentTopology<'_>) -> Result<(), TopologyAuthoringError> {
        *self.0.borrow_mut() = Some(ctx.output("out", 0)?);
        ctx.output::<Payload>("out", 1)?;
        Ok(())
    }
}
#[test]
fn failed_component_is_atomic_and_leaked_handles_stay_invalid() {
    let mut app = TopologyBuilder::new("application/a").unwrap();
    let enclave = app.enclave("a").unwrap();
    let leaked = RefCell::new(None);
    assert!(app.component("a", Failed(&leaked), &enclave).is_err());
    let a = app.component("a", Definition, &enclave).unwrap();
    assert!(matches!(
        app.connect(leaked.borrow().as_ref().unwrap(), &a.0),
        Err(TopologyAuthoringError::ForeignHandle)
    ));
    let topology = app.finish().unwrap();
    assert_eq!(topology.ports().count(), 2);
    assert_eq!(topology.components().count(), 1);
}
#[test]
fn invalid_names_and_finish_validation_are_returned() {
    assert!(matches!(
        TopologyBuilder::new(""),
        Err(TopologyAuthoringError::InvalidStableId(_))
    ));
    let mut app = TopologyBuilder::new("app").unwrap();
    assert!(matches!(
        app.enclave(""),
        Err(TopologyAuthoringError::InvalidStableId(_))
    ));
    app.enclave("missing").unwrap();
    assert!(matches!(
        app.finish(),
        Err(TopologyAuthoringError::Topology(_))
    ));
}
