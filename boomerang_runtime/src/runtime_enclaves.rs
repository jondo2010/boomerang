//! Owned, key-preserving collections of scheduler Enclaves.

use std::ops::{Index, IndexMut};

use crate::{Enclave, EnclaveKey};

/// A self-contained set of locally communicating Enclaves.
///
/// The sparse backing map preserves globally allocated [`EnclaveKey`] values when a lowered
/// assembly is split into independently movable Federates.
#[derive(Default)]
pub struct RuntimeEnclaves {
    enclaves: tinymap::TinySecondaryMap<EnclaveKey, Enclave>,
    next_key: usize,
    #[cfg(feature = "replay")]
    replayers: crate::replay::ReplayersMap,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeEnclavesError {
    #[error("Enclave {0:?} is assigned to more than one runtime group")]
    DuplicateOwner(EnclaveKey),
    #[error("Enclave {0:?} has no runtime group")]
    MissingOwner(EnclaveKey),
    #[error("runtime group references unknown Enclave {0:?}")]
    UnknownEnclave(EnclaveKey),
}

impl std::fmt::Debug for RuntimeEnclaves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeEnclaves")
            .field("enclaves", &self.enclaves)
            .finish_non_exhaustive()
    }
}

impl RuntimeEnclaves {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, enclave: Enclave) -> EnclaveKey {
        let key = EnclaveKey::from(self.next_key);
        self.next_key += 1;
        assert!(self.enclaves.insert(key, enclave).is_none());
        key
    }

    pub fn insert_at(&mut self, key: EnclaveKey, enclave: Enclave) {
        assert!(self.enclaves.insert(key, enclave).is_none());
        self.next_key = self.next_key.max(tinymap::Key::index(&key) + 1);
    }

    pub fn get(&self, key: EnclaveKey) -> Option<&Enclave> {
        self.enclaves.get(key)
    }

    pub fn get_mut(&mut self, key: EnclaveKey) -> Option<&mut Enclave> {
        self.enclaves.get_mut(key)
    }

    pub fn len(&self) -> usize {
        self.enclaves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enclaves.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = EnclaveKey> + '_ {
        self.enclaves.keys()
    }

    pub fn values(&self) -> impl Iterator<Item = &Enclave> {
        self.enclaves.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Enclave> {
        self.enclaves.iter_mut().map(|(_, enclave)| enclave)
    }

    pub fn iter(&self) -> impl Iterator<Item = (EnclaveKey, &Enclave)> {
        self.enclaves.iter()
    }

    /// Split this collection without changing any Enclave keys.
    pub fn split_by<G>(
        self,
        groups: impl IntoIterator<Item = (G, Vec<EnclaveKey>)>,
    ) -> Result<std::collections::BTreeMap<G, Self>, RuntimeEnclavesError>
    where
        G: Ord + Clone,
    {
        let mut owners = tinymap::TinySecondaryMap::<EnclaveKey, G>::new();
        let mut result = std::collections::BTreeMap::new();
        for (group, keys) in groups {
            result.entry(group.clone()).or_insert_with(Self::new);
            for key in keys {
                if !self.enclaves.contains_key(key) {
                    return Err(RuntimeEnclavesError::UnknownEnclave(key));
                }
                if owners.insert(key, group.clone()).is_some() {
                    return Err(RuntimeEnclavesError::DuplicateOwner(key));
                }
            }
        }

        let Self {
            enclaves,
            #[cfg(feature = "replay")]
            replayers,
            ..
        } = self;
        for (key, enclave) in enclaves {
            let Some(owner) = owners.get(key) else {
                if enclave.env.reactions.is_empty() {
                    continue;
                }
                return Err(RuntimeEnclavesError::MissingOwner(key));
            };
            result
                .get_mut(owner)
                .expect("owner groups are created with their assignment")
                .insert_at(key, enclave);
        }
        #[cfg(feature = "replay")]
        for (key, enclave_replayers) in replayers {
            let Some(owner) = owners.get(key) else {
                continue;
            };
            result
                .get_mut(owner)
                .expect("owner groups are created with their assignment")
                .replayers
                .insert(key, enclave_replayers);
        }
        Ok(result)
    }

    #[cfg(not(feature = "replay"))]
    pub fn into_parts(self) -> tinymap::TinySecondaryMap<EnclaveKey, Enclave> {
        self.enclaves
    }

    #[cfg(feature = "replay")]
    pub fn into_parts(
        self,
    ) -> (
        tinymap::TinySecondaryMap<EnclaveKey, Enclave>,
        crate::replay::ReplayersMap,
    ) {
        (self.enclaves, self.replayers)
    }

    #[cfg(feature = "replay")]
    pub fn replayers_mut(&mut self) -> &mut crate::replay::ReplayersMap {
        &mut self.replayers
    }

    #[cfg(feature = "replay")]
    pub fn take_replayers(&mut self) -> crate::replay::ReplayersMap {
        std::mem::take(&mut self.replayers)
    }

    #[cfg(feature = "replay")]
    pub fn into_replayers(self) -> crate::replay::ReplayersMap {
        self.replayers
    }
}

impl Index<EnclaveKey> for RuntimeEnclaves {
    type Output = Enclave;

    fn index(&self, key: EnclaveKey) -> &Self::Output {
        &self.enclaves[key]
    }
}

impl IndexMut<EnclaveKey> for RuntimeEnclaves {
    fn index_mut(&mut self, key: EnclaveKey) -> &mut Self::Output {
        &mut self.enclaves[key]
    }
}

impl IntoIterator for RuntimeEnclaves {
    type Item = (EnclaveKey, Enclave);
    type IntoIter = tinymap::secondary_map::IntoIter<EnclaveKey, Enclave>;

    fn into_iter(self) -> Self::IntoIter {
        self.enclaves.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_preserves_enclave_keys() {
        let mut enclaves = RuntimeEnclaves::new();
        let first = enclaves.insert(Enclave::default());
        let second = enclaves.insert(Enclave::default());
        let third = enclaves.insert(Enclave::default());

        let groups = enclaves
            .split_by([("a", vec![first, third]), ("b", vec![second])])
            .unwrap();

        assert_eq!(groups["a"].keys().collect::<Vec<_>>(), vec![first, third]);
        assert_eq!(groups["b"].keys().collect::<Vec<_>>(), vec![second]);
    }

    #[test]
    fn split_rejects_multiple_owners_for_one_enclave() {
        let mut enclaves = RuntimeEnclaves::new();
        let key = enclaves.insert(Enclave::default());

        let error = enclaves
            .split_by([("a", vec![key]), ("b", vec![key])])
            .expect_err("one Enclave cannot belong to two runtime groups");

        assert!(matches!(error, RuntimeEnclavesError::DuplicateOwner(found) if found == key));
    }

    #[test]
    fn split_rejects_unknown_enclave_assignment() {
        let enclaves = RuntimeEnclaves::new();
        let unknown = EnclaveKey::from(7);

        let error = enclaves
            .split_by([("a", vec![unknown])])
            .expect_err("placement cannot reference an Enclave outside the runtime");

        assert!(matches!(error, RuntimeEnclavesError::UnknownEnclave(found) if found == unknown));
    }
}
