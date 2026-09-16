//! Typed composition of compiler declarations without constructing runtime state.
use super::*;
use std::{collections::BTreeMap, marker::PhantomData, sync::Arc};

/// Failure while authoring or validating an application topology.
#[derive(Debug, thiserror::Error)]
pub enum TopologyAuthoringError {
    /// A supplied identity is invalid.
    #[error(transparent)]
    InvalidStableId(#[from] InvalidStableId),
    /// The authoritative topology builder rejected a declaration.
    #[error(transparent)]
    Topology(#[from] TopologyBuildError),
    /// A handle belongs to another builder or an unsuccessful declaration.
    #[error("topology handle belongs to another builder or an unsuccessful declaration")]
    ForeignHandle,
}

/// A component's target-neutral declarations and typed public port surface.
pub trait ComponentDefinition {
    /// Public ports returned after successful declaration.
    type Ports;
    /// Stable component contract and version.
    fn contract(&self) -> (&str, u64);
    /// Declare component members; no runtime state is required.
    fn declare(
        &self,
        topology: &mut ComponentTopology<'_>,
    ) -> Result<Self::Ports, TopologyAuthoringError>;
}

/// An enclave declared by one particular authoring builder.
#[derive(Clone, Debug)]
pub struct TopologyEnclave {
    owner: Arc<()>,
    id: StableEnclaveId,
    placement: PlacementGroupId,
}

/// A typed input port. Handles do not require any payload runtime traits.
#[derive(Debug)]
pub struct TopologyInput<T> {
    id: PortId,
    declaration: Arc<()>,
    payload: PhantomData<fn(T) -> T>,
}
/// A typed output port. Handles do not require any payload runtime traits.
#[derive(Debug)]
pub struct TopologyOutput<T> {
    id: PortId,
    declaration: Arc<()>,
    payload: PhantomData<fn(T) -> T>,
}
macro_rules! port_handle {
    ($handle:ident) => {
        impl<T> Clone for $handle<T> {
            fn clone(&self) -> Self {
                Self {
                    id: self.id.clone(),
                    declaration: self.declaration.clone(),
                    payload: PhantomData,
                }
            }
        }
        impl<T> $handle<T> {
            /// Stable identity of the declared port.
            pub fn id(&self) -> &PortId {
                &self.id
            }
        }
    };
}
port_handle!(TopologyInput);
port_handle!(TopologyOutput);

/// Transactional convenience layer over [`ApplicationTopologyBuilder`].
///
/// Connections require matching payload types and output-to-input direction:
/// ```compile_fail
/// use boomerang_builder::compiler::*;
/// fn mismatched(app: &mut TopologyBuilder, out: &TopologyOutput<u32>, input: &TopologyInput<String>) {
///     app.connect(out, input).unwrap();
/// }
/// ```
/// ```compile_fail
/// use boomerang_builder::compiler::*;
/// fn reversed(app: &mut TopologyBuilder, input: &TopologyInput<u32>, out: &TopologyOutput<u32>) {
///     app.connect(input, out).unwrap();
/// }
/// ```
#[derive(Debug)]
pub struct TopologyBuilder {
    builder: ApplicationTopologyBuilder,
    owner: Arc<()>,
    ports: BTreeMap<PortId, Arc<()>>,
    connections: BTreeMap<(PortId, PortId), u32>,
}
impl TopologyBuilder {
    /// Begin an application using its canonical stable identity.
    pub fn new(application: impl Into<Box<str>>) -> Result<Self, TopologyAuthoringError> {
        Ok(Self {
            builder: ApplicationTopologyBuilder::new(application)?,
            owner: Arc::new(()),
            ports: BTreeMap::new(),
            connections: BTreeMap::new(),
        })
    }
    /// Declare a shared enclave whose root component will have the given name.
    pub fn enclave(&mut self, root_name: &str) -> Result<TopologyEnclave, TopologyAuthoringError> {
        let path = StablePath::from_name(root_name)?;
        let id = StableEnclaveId::from_path(path.clone());
        let placement = PlacementGroupId::from_path(
            StablePath::from_name("placement")?.append_name(path.to_string())?,
        );
        let mut staged = self.builder.clone();
        staged.add_enclave(id.clone(), ReactorId::from_path(path))?;
        staged.add_placement_group(placement.clone(), None)?;
        self.builder = staged;
        Ok(TopologyEnclave {
            owner: self.owner.clone(),
            id,
            placement,
        })
    }
    /// Declare a component atomically, returning its typed ports on success.
    pub fn component<D: ComponentDefinition>(
        &mut self,
        name: &str,
        definition: D,
        enclave: &TopologyEnclave,
    ) -> Result<D::Ports, TopologyAuthoringError> {
        if !Arc::ptr_eq(&self.owner, &enclave.owner) {
            return Err(TopologyAuthoringError::ForeignHandle);
        }
        let path = StablePath::from_name(name)?;
        let root = ReactorId::from_path(path.clone());
        let component = ComponentInstanceId::from_path(path);
        let (contract, version) = definition.contract();
        let mut staged = self.builder.clone();
        staged.add_component(ComponentInstance::from_ids(
            component.clone(),
            ContractId::new(contract)?,
            version,
        ))?;
        staged.add_reactor(Reactor::new(
            root.clone(),
            component,
            None,
            None,
            enclave.id.clone(),
            Some(enclave.placement.clone()),
            None,
        ))?;
        for (position, name, kind) in [
            (0, "__startup", ActionKind::Startup),
            (1, "__shutdown", ActionKind::Shutdown),
        ] {
            staged.add_action(
                ActionId::from_path(root.path().append_name(name)?),
                root.clone(),
                kind,
                position,
                None,
            )?;
        }
        let mut context = ComponentTopology {
            builder: &mut staged,
            root,
            declaration: Arc::new(()),
            ports: BTreeMap::new(),
        };
        let result = definition.declare(&mut context)?;
        self.ports.append(&mut context.ports);
        self.builder = staged;
        Ok(result)
    }
    /// Connect matching payloads with an immediate logical connection.
    pub fn connect<T>(
        &mut self,
        output: &TopologyOutput<T>,
        input: &TopologyInput<T>,
    ) -> Result<(), TopologyAuthoringError> {
        for (id, declaration) in [
            (&output.id, &output.declaration),
            (&input.id, &input.declaration),
        ] {
            if !self
                .ports
                .get(id)
                .is_some_and(|accepted| Arc::ptr_eq(accepted, declaration))
            {
                return Err(TopologyAuthoringError::ForeignHandle);
            }
        }
        let pair = (output.id.clone(), input.id.clone());
        let ordinal = self.connections.get(&pair).copied().unwrap_or(0);
        let path = StablePath::from_name("boundary")?
            .append_name(output.id.to_canonical_string())?
            .append_name(input.id.to_canonical_string())?
            .append_name(format!("c{ordinal}"))?;
        self.builder.add_connection(
            BoundaryId::from_path(path),
            output.id.clone(),
            input.id.clone(),
            ConnectionSemantics::Logical { after: None },
        )?;
        // No graph can contain enough connections to exhaust this counter in memory.
        self.connections.insert(pair, ordinal + 1);
        Ok(())
    }
    /// Validate and produce the authoritative canonical topology.
    pub fn finish(self) -> Result<ApplicationTopology, TopologyAuthoringError> {
        Ok(self.builder.finish()?)
    }
}

/// Declaration context for one component, backed by the compiler's existing builder.
pub struct ComponentTopology<'a> {
    builder: &'a mut ApplicationTopologyBuilder,
    root: ReactorId,
    declaration: Arc<()>,
    ports: BTreeMap<PortId, Arc<()>>,
}
impl ComponentTopology<'_> {
    /// Stable identity of this component's root reactor.
    pub fn root(&self) -> &ReactorId {
        &self.root
    }
    /// Existing low-level declaration builder; final validation remains authoritative.
    pub fn builder(&mut self) -> &mut ApplicationTopologyBuilder {
        self.builder
    }
    fn port(
        &mut self,
        name: &str,
        direction: PortDirection,
        position: u32,
    ) -> Result<PortId, TopologyAuthoringError> {
        let id = PortId::from_path(self.root.path().append_name(name)?);
        self.builder.add_port(
            id.clone(),
            self.root.clone(),
            direction,
            None,
            position,
            None,
        )?;
        self.ports.insert(id.clone(), self.declaration.clone());
        Ok(id)
    }
    /// Declare a scalar input and return its typed handle.
    pub fn input<T>(
        &mut self,
        name: &str,
        position: u32,
    ) -> Result<TopologyInput<T>, TopologyAuthoringError> {
        Ok(TopologyInput {
            id: self.port(name, PortDirection::Input, position)?,
            declaration: self.declaration.clone(),
            payload: PhantomData,
        })
    }
    /// Declare a scalar output and return its typed handle.
    pub fn output<T>(
        &mut self,
        name: &str,
        position: u32,
    ) -> Result<TopologyOutput<T>, TopologyAuthoringError> {
        Ok(TopologyOutput {
            id: self.port(name, PortDirection::Output, position)?,
            declaration: self.declaration.clone(),
            payload: PhantomData,
        })
    }
}
