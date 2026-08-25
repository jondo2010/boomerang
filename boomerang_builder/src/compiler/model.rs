use std::{collections::BTreeMap, fmt};

use super::{ApplicationId, ComponentInstanceId, ContractId, InvalidStableId};
use tinymap::{Key, TinyMap};

/// Dense component key whose lifetime is limited to one topology representation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ComponentKey(
    /// Dense component index assigned after stable-ID canonicalization.
    usize,
);

impl From<usize> for ComponentKey {
    fn from(index: usize) -> Self {
        Self(index)
    }
}

impl Key for ComponentKey {
    fn index(&self) -> usize {
        self.0
    }
}

/// A logical component instance and the contract it requires.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentInstance {
    /// Stable application-level identity of this instance.
    id: ComponentInstanceId,
    /// Stable identity of the instance's external contract.
    contract: ContractId,
}

impl ComponentInstance {
    /// Creates a component instance from stable component and contract identities.
    pub fn new(
        id: impl Into<Box<str>>,
        contract: impl Into<Box<str>>,
    ) -> Result<Self, InvalidStableId> {
        Ok(Self {
            id: ComponentInstanceId::new(id)?,
            contract: ContractId::new(contract)?,
        })
    }

    /// Returns this component's stable application identity.
    pub fn id(&self) -> &ComponentInstanceId {
        &self.id
    }

    /// Returns this component's required contract identity.
    pub fn contract(&self) -> &ContractId {
        &self.contract
    }
}

/// Reports a structural error while staging an application topology.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TopologyBuildError {
    /// A component stable identity was declared more than once.
    #[error("duplicate component identity '{component_id}'")]
    DuplicateComponentId {
        /// Stable identity shared by the conflicting declarations.
        component_id: ComponentInstanceId,
    },
}

/// Stages logical application declarations by stable identity before dense interning.
#[derive(Debug)]
pub struct ApplicationTopologyBuilder {
    /// Stable identity of the application being assembled.
    application_id: ApplicationId,
    /// Component declarations indexed and ordered by their stable identities.
    staged_components: BTreeMap<ComponentInstanceId, ComponentInstance>,
}

/// Immutable, target-neutral logical application structure.
pub struct ApplicationTopology {
    /// Stable identity of this application.
    application_id: ApplicationId,
    /// Canonically interned component records keyed only within this representation.
    components: TinyMap<ComponentKey, ComponentInstance>,
    /// Stable component identities mapped to representation-local dense keys for boundary lookup.
    component_keys_by_id: BTreeMap<ComponentInstanceId, ComponentKey>,
}

impl fmt::Debug for ApplicationTopology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationTopology")
            .field("application_id", &self.application_id)
            .field("components", &self.components().collect::<Vec<_>>())
            .finish()
    }
}

impl ApplicationTopologyBuilder {
    /// Creates an empty topology builder for a stable application identity.
    pub fn new(application_id: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
        Ok(Self {
            application_id: ApplicationId::new(application_id)?,
            staged_components: BTreeMap::new(),
        })
    }

    /// Stages one component, rejecting a repeated stable component identity.
    pub fn add_component(
        &mut self,
        component: ComponentInstance,
    ) -> Result<(), TopologyBuildError> {
        let component_id = component.id.clone();
        if self.staged_components.contains_key(&component_id) {
            return Err(TopologyBuildError::DuplicateComponentId { component_id });
        }
        self.staged_components.insert(component_id, component);
        Ok(())
    }

    /// Canonicalizes declarations by stable ID and interns them into an immutable topology.
    pub fn finish(self) -> ApplicationTopology {
        let mut components = TinyMap::with_capacity(self.staged_components.len());
        let mut component_keys_by_id = BTreeMap::new();
        for (component_id, component) in self.staged_components {
            let key = components.insert(component);
            component_keys_by_id.insert(component_id, key);
        }
        ApplicationTopology {
            application_id: self.application_id,
            components,
            component_keys_by_id,
        }
    }
}

impl ApplicationTopology {
    /// Returns this topology's stable application identity.
    pub fn application_id(&self) -> &ApplicationId {
        &self.application_id
    }

    /// Iterates stable component identities and records in stable identity order.
    pub fn components(&self) -> impl Iterator<Item = (&ComponentInstanceId, &ComponentInstance)> {
        self.component_keys_by_id
            .iter()
            .map(|(component_id, key)| (component_id, &self.components[*key]))
    }

    /// Looks up a component record by its stable identity.
    pub fn component(&self, component_id: &ComponentInstanceId) -> Option<&ComponentInstance> {
        self.component_keys_by_id
            .get(component_id)
            .and_then(|key| self.components.get(*key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_declarations_intern_to_stable_dense_order() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        for suffix in ["c", "b", "a"] {
            builder
                .add_component(
                    ComponentInstance::new(format!("vehicle/{suffix}"), "sensor.v1").unwrap(),
                )
                .unwrap();
        }

        let topology = builder.finish();
        let dense_indices = ["a", "b", "c"].map(|suffix| {
            let id = ComponentInstanceId::new(format!("vehicle/{suffix}")).unwrap();
            topology.component_keys_by_id[&id].0
        });

        assert_eq!(dense_indices, [0, 1, 2]);
        assert_eq!(
            topology
                .components()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            ["vehicle/a", "vehicle/b", "vehicle/c"]
        );
    }
}
