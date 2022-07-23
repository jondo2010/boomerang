use itertools::Itertools;
use slotmap::{Key, SecondaryMap};

use crate::Level;

/// A set of Keys organized by level
#[derive(Debug, Clone, Default)]
pub struct KeySet<K: Key> {
    /// List of SecondaryMaps, reverse-sorted by Level
    levels: Vec<(Level, SecondaryMap<K, ()>)>,
}

impl<K: Key> KeySet<K> {
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    /// Build the levels structure from an iterable
    fn build_levels<I>(keys: I) -> Vec<(usize, SecondaryMap<K, ()>)>
    where
        I: IntoIterator<Item = (Level, K)>,
    {
        keys
            .into_iter()
            .sorted_by_key(|(level, _)| -(*level as isize))
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
        Self { levels: Self::build_levels(keys) }
    }

    /// Extend the set from `other` into `self`. Any keys at a level lower than the current
    /// `min_level` are ignored.
    pub fn extend_above<I>(&mut self, iter: I, min_level: usize)
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

impl<K> Iterator for KeySet<K>
where
    K: Key,
{
    type Item = (Level, impl ExactSizeIterator<Item = K>);

    /// Provides the next lowest level in the set.
    fn next(&mut self) -> Option<Self::Item> {
        self.levels
            .pop()
            .map(|(level, keys)| (level, keys.into_iter().map(|(k, _)| k)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn test_set1() {
        let mut sm = SlotMap::new();
        let key0 = sm.insert(0);
        let key1 = sm.insert(2);
        let key2 = sm.insert(1);
        let key3 = sm.insert(1);

        let vals = sm.iter().map(|(k, v)| (*v, k));

        let mut rset = KeySet::from_iter(vals);
        assert_eq!(rset.levels.len(), 3);
        let levels = rset.levels.iter().map(|level| level.0).collect_vec();
        assert_eq!(levels, vec![2, 1, 0]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, 0);
        assert_eq!(keys.collect_vec(), vec![key0]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, 1);
        assert_eq!(keys.collect_vec(), vec![key2, key3]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, 2);
        assert_eq!(keys.collect_vec(), vec![key1]);
    }

    #[test]
    fn test_set2() {
        let mut sm = SlotMap::new();
        let key0 = sm.insert(());
        let key1 = sm.insert(());
        let key2 = sm.insert(());

        let vals1 = IntoIterator::into_iter([(2usize, key0), (1usize, key1), (2, key2)]);
        let mut rset = KeySet::from_iter(vals1);

        assert_eq!(rset.levels.len(), 2);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, 1);
        assert_eq!(keys.collect_vec(), vec![key1]);

        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, 2);
        assert_eq!(keys.collect_vec(), vec![key0, key2]);
    }

    #[test]
    fn test_set3() {
        let mut sm = SlotMap::new();
        let key0 = sm.insert(());
        let key1 = sm.insert(());
        let key2 = sm.insert(());
        let key3 = sm.insert(());

        let mut rset = KeySet::default();

        // Extend into empty set
        rset.extend_above(IntoIterator::into_iter([(1usize, key0)]), 0);
        assert_eq!(rset.levels.len(), 1);

        // Extend into existing group
        rset.extend_above(IntoIterator::into_iter([(1usize, key1)]), 0);
        assert_eq!(rset.levels.len(), 1);
        assert_eq!(rset.levels[0].1.len(), 2);

        // Extend before existing group
        rset.extend_above(IntoIterator::into_iter([(0usize, key2)]), 1);
        assert_eq!(
            rset.levels.len(),
            1,
            "Extending with keys in level before existing level should be ignored."
        );

        // Extend after existing group
        rset.extend_above(IntoIterator::into_iter([(2usize, key3)]), 1);
        assert_eq!(rset.levels.len(), 2);
    }

    #[test]
    fn test_set4() {
        let mut sm = SlotMap::new();
        let key0 = sm.insert(());
        let key1 = sm.insert(());
        let key2 = sm.insert(());
        let key3 = sm.insert(());

        let mut rset = KeySet::default();

        rset.extend_above([(0, key0)].into_iter(), 0);
        let (level, keys) = rset.next().unwrap();
        assert_eq!(level, 0);
        assert_eq!(keys.collect_vec(), vec![key0]);

        rset.extend_above([(3, key1), (1, key2)].into_iter(), 1);
        assert_eq!(rset.levels.len(), 2);

        // Should be (1, key2)
        let (level, keys) = rset.next().unwrap();
        assert_eq!((level, keys.collect_vec()), (1, vec![key2]));
        assert_eq!(rset.levels.len(), 1);

        rset.extend_above([(2, key3)].into_iter(), 1);
        let (level, keys) = rset.next().unwrap();
        assert_eq!((level, keys.collect_vec()), (2, vec![key3]));
    }
}
