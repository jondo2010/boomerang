//! Serde serialization and deserialization for `KeySet`.
//!
//! We provide a custom implementation for `KeySet` serialization and deserialization instead of
//! exposing the internal representation of the set.
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use super::*;

impl<K> Serialize for KeySet<K>
where
    K: Key + Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

struct KeySetVisitor<K: Key> {
    marker: PhantomData<fn() -> KeySet<K>>,
}

impl<K: Key> Default for KeySetVisitor<K> {
    fn default() -> Self {
        Self {
            marker: Default::default(),
        }
    }
}

impl<'de, K> de::Visitor<'de> for KeySetVisitor<K>
where
    K: Key + Deserialize<'de>,
{
    type Value = KeySet<K>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("set")
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        // First deserialize the sequence into a vec since we need to know the size.
        let mut v = Vec::<K>::with_capacity(access.size_hint().unwrap_or(0));
        while let Some(key) = access.next_element()? {
            v.push(key);
        }
        Ok(KeySet::from_iter(v))
    }
}

impl<'de, K> Deserialize<'de> for KeySet<K>
where
    K: Key + Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(KeySetVisitor::default())
    }
}
