use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use super::*;

impl<K, V> Serialize for TinySecondaryMap<K, V>
where
    K: Key + Serialize,
    V: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter())
    }
}

struct TinySecondaryMapVisitor<K: Key, V> {
    marker: PhantomData<fn() -> TinySecondaryMap<K, V>>,
}

impl<K: Key, V> Default for TinySecondaryMapVisitor<K, V> {
    fn default() -> Self {
        Self {
            marker: Default::default(),
        }
    }
}

impl<'de, K, V> de::Visitor<'de> for TinySecondaryMapVisitor<K, V>
where
    K: Key + Deserialize<'de>,
    V: Deserialize<'de>,
{
    type Value = TinySecondaryMap<K, V>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("map")
    }

    fn visit_map<A: de::MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut map = Self::Value::with_capacity(access.size_hint().unwrap_or(0));
        while let Some((key, value)) = access.next_entry()? {
            map.insert(key, value);
        }
        Ok(map)
    }
}

impl<'de, K, V> Deserialize<'de> for TinySecondaryMap<K, V>
where
    K: Key + Deserialize<'de>,
    V: Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(TinySecondaryMapVisitor::default())
    }
}
