use std::fmt::Debug;

use itertools::Itertools;

use crate::Level;

/// A set of Keys organized by [`Level`]. The set is sorted by `Level` in descending order, so that the highest level
/// is always at the front of the set. Calling `next()` will pop the lowest level off the end.
#[derive(Clone, Default)]
pub struct KeySet<K: tinymap::Key> {
    /// List of SecondaryMaps, reverse-sorted by Level
    levels: Vec<(Level, tinymap::TinySecondaryMap<K, ()>)>,
}

impl<K: tinymap::Key + Debug> Debug for KeySet<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(
                self.levels
                    .iter()
                    .map(|(level, keys)| (level, keys.clone().into_keys())),
            )
            .finish()
    }
}

impl<K: tinymap::Key> KeySet<K> {
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    /// Build the levels structure from an iterable
    #[inline]
    fn build_levels<I>(keys: I) -> Vec<(Level, tinymap::TinySecondaryMap<K, ()>)>
    where
        I: IntoIterator<Item = (Level, K)>,
    {
        keys.into_iter()
            .sorted_by_key(|(level, _)| -(level.0 as isize))
            .group_by(|(level, _)| *level)
            .into_iter()
            .map(|(level, group)| (level, group.map(|(_, key)| (key, ())).collect()))
            .collect_vec()
    }

    /// Create a new KeySet from an iterable
    pub fn from_iter<I>(keys: I) -> Self
    where
        I: IntoIterator<Item = (Level, K)>,
    {
        Self {
            levels: Self::build_levels(keys),
        }
    }

    /// Extend the set from `other` into `self`. Any keys at a level lower than the current
    /// `min_level` are ignored.
    pub fn extend_above<I>(&mut self, iter: I, min_level: Level)
    where
        I: IntoIterator<Item = (Level, K)>,
    {
        if self.levels.is_empty() {
            // Special case
            self.levels = Self::build_levels(iter);
        } else {
            for (new_level, new_key) in iter.into_iter() {
                if new_level >= min_level {
                    match self
                        .levels
                        .iter()
                        .find_position(|(level, _)| level <= &new_level)
                    {
                        Some((pos, (level, _))) if level == &new_level => {
                            // Add to existing level
                            self.levels[pos].1.insert(new_key, ());
                        }
                        Some((pos, _)) => {
                            // Insert before existing level
                            let keys = IntoIterator::into_iter([(new_key, ())]).collect();
                            self.levels.insert(pos, (new_level, keys));
                        }
                        None => {
                            // Push to the back
                            let keys = IntoIterator::into_iter([(new_key, ())]).collect();
                            self.levels.push((new_level, keys));
                        }
                    }
                }
            }
        }
    }
}

impl<K: tinymap::Key> FromIterator<(Level, K)> for KeySet<K> {
    #[inline]
    fn from_iter<T: IntoIterator<Item = (Level, K)>>(iter: T) -> Self {
        Self {
            levels: Self::build_levels(iter),
        }
    }
}

impl<K: tinymap::Key> Iterator for KeySet<K> {
    type Item = (Level, Vec<K>);

    /// Provides the next lowest level in the set.
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.levels
            .pop()
            .map(|(level, keys)| (level, keys.into_keys()))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.levels.len(), Some(self.levels.len()))
    }
}

#[cfg(test)]
mod tests {
    use tinymap::DefaultKey;

    use super::*;

    #[test]
    fn test_set1() {
        let mut map = tinymap::TinyMap::<DefaultKey, _>::new();
        let key0 = map.insert(0);
        let key1 = map.insert(2);
        let key2 = map.insert(1);
        let key3 = map.insert(1);

        let vals = map.iter().map(|(k, v)| (Level(*v), k));

        let mut rset = KeySet::from_iter(vals);
        assert_eq!(rset.levels.len(), 3);
        let levels = rset.levels.iter().map(|level| level.0).collect_vec();
        assert_eq!(levels, vec![Level(2), Level(1), Level(0)]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, Level(0));
        assert_eq!(keys, vec![key0]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, Level(1));
        assert_eq!(keys, vec![key2, key3]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, Level(2));
        assert_eq!(keys, vec![key1]);
    }

    #[test]
    fn test_set2() {
        let mut map = tinymap::TinyMap::<DefaultKey, _>::new();
        let key0 = map.insert(());
        let key1 = map.insert(());
        let key2 = map.insert(());

        let vals1 = IntoIterator::into_iter([(Level(2), key0), (Level(1), key1), (Level(2), key2)]);
        let mut rset = KeySet::from_iter(vals1);

        assert_eq!(rset.levels.len(), 2);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, Level(1));
        assert_eq!(keys, vec![key1]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, Level(2));
        assert_eq!(keys, vec![key0, key2]);
    }

    #[test]
    fn test_set3() {
        let mut map = tinymap::TinyMap::<DefaultKey, _>::new();
        let key0 = map.insert(());
        let key1 = map.insert(());
        let key2 = map.insert(());
        let key3 = map.insert(());

        let mut rset = KeySet::default();

        // Extend into empty set
        rset.extend_above(IntoIterator::into_iter([(Level(1), key0)]), Level(0));
        assert_eq!(rset.levels.len(), 1);

        // Extend into existing group
        rset.extend_above(IntoIterator::into_iter([(Level(1), key1)]), Level(0));
        assert_eq!(rset.levels.len(), 1);
        assert_eq!(rset.levels[0].1.len(), 2);

        // Extend before existing group
        rset.extend_above(IntoIterator::into_iter([(Level(0), key2)]), Level(1));
        assert_eq!(
            rset.levels.len(),
            1,
            "Extending with keys in level before existing level should be ignored."
        );

        // Extend after existing group
        rset.extend_above(IntoIterator::into_iter([(Level(2), key3)]), Level(1));
        assert_eq!(rset.levels.len(), 2);
    }

    #[test]
    fn test_set4() {
        let mut map = tinymap::TinyMap::<DefaultKey, _>::new();
        let key0 = map.insert(());
        let key1 = map.insert(());
        let key2 = map.insert(());
        let key3 = map.insert(());

        let mut rset = KeySet::default();

        rset.extend_above([(Level(0), key0)], Level(0));
        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, Level(0));
        assert_eq!(keys, vec![key0]);

        rset.extend_above([(Level(3), key1), (Level(1), key2)], Level(1));
        assert_eq!(rset.levels.len(), 2);

        // Should be (1, key2)
        let (level, keys) = rset.next().unwrap();
        assert_eq!((level, keys), (Level(1), vec![key2]));
        assert_eq!(rset.levels.len(), 1);

        rset.extend_above([(Level(2), key3)], Level(1));
        let (level, keys) = rset.next().unwrap();
        assert_eq!((level, keys), (Level(2), vec![key3]));
    }
}
