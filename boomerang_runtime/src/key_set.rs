use crate::Level;

impl tinymap::Key for Level {
    fn index(&self) -> usize {
        self.0
    }
}

/// Sets of [`tinymap::Key`]s indexed by [`Level`].
#[derive(Default, Debug, Clone)]
pub struct KeySet<K: tinymap::Key> {
    /// The set of keys at each level.
    levels: Vec<tinymap::TinySecondarySet<K>>,
}

impl<K: tinymap::Key + std::fmt::Display> std::fmt::Display for KeySet<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(
                self.levels
                    .iter()
                    .enumerate()
                    .map(|(level, keys)| (Level(level), keys.to_string())),
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct KeySetLimits {
    /// The maximum level of any reaction in the trigger map.
    pub max_level: Level,
    /// The total number of reactions in the trigger map.
    pub num_keys: usize,
}

impl<K: tinymap::Key> KeySet<K> {
    /// Create a new KeySet with a fixed number of levels and key capacity.
    pub fn new(
        KeySetLimits {
            max_level,
            num_keys,
        }: &KeySetLimits,
    ) -> Self {
        Self {
            levels: vec![tinymap::TinySecondarySet::with_capacity(*num_keys); max_level.0 + 1],
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    /// Extend the levels structure from an iterable
    pub fn extend_above(&mut self, keys: impl IntoIterator<Item = (Level, K)>) {
        for (level, key) in keys.into_iter() {
            let idx = level.0;
            self.levels[idx].insert(key);
        }
    }

    /// Returns a view of the levels in the set, starting at level 0.
    pub fn view(&mut self) -> KeySetView<'_, K> {
        KeySetView {
            levels: self.levels.as_mut_slice(),
            current_level: Level(0),
        }
    }

    /// Clear all keys from all levels.
    pub fn clear(&mut self) {
        for level in self.levels.iter_mut() {
            level.clear();
        }
    }
}

pub struct KeySetView<'a, K: tinymap::Key> {
    levels: &'a mut [tinymap::TinySecondarySet<K>],
    /// Indicates the implicit level at levels[0].
    current_level: Level,
}

impl<'a, K: tinymap::Key> KeySetView<'a, K> {
    /// Returns true if there are remaining levels to process.
    pub fn levels_remaining(&self) -> bool {
        self.levels[self.current_level.0..]
            .iter()
            .any(|level| !level.is_empty())
    }

    /// Executes the provided closure on each non-empty level in the set.
    ///
    /// The closure is passed the current level, the keys at that level, and a
    /// mutable reference to the remaining levels. Empty levels are skipped.
    pub fn for_each_level<F>(mut self, mut f: F)
    where
        F: FnMut(Level, tinymap::secondary_set::Iter<'_, K>, Option<KeySetViewMut<'_, K>>),
    {
        while self.current_level.0 < self.levels.len() {
            // skip empty levels
            if self.levels[self.current_level.0].is_empty() {
                self.current_level += 1;
                continue;
            }

            // split the levels into the first and the rest, starting at the current level
            let (upper, lower) = self.levels.split_at_mut(self.current_level.0 + 1);
            assert!(!upper.is_empty());

            let remaining = (!lower.is_empty()).then(|| KeySetViewMut {
                levels: lower,
                current_level: self.current_level + 1,
            });

            let first = &upper[self.current_level.0];
            f(self.current_level, first.iter(), remaining);

            self.current_level += 1;
        }
    }
}

pub struct KeySetViewMut<'a, K: tinymap::Key> {
    levels: &'a mut [tinymap::TinySecondarySet<K>],
    /// Indicates the implicit level at levels[0].
    current_level: Level,
}

impl<'a, K: tinymap::Key> KeySetViewMut<'a, K> {
    /// Extend the levels structure from an iterable, inserting keys at levels above the current level.
    pub fn extend_above(&mut self, keys: impl IntoIterator<Item = (Level, K)>) {
        for (level, key) in keys.into_iter() {
            if level >= self.current_level {
                // The current level corresponds to index 0
                let idx = level.0 - self.current_level.0;
                self.levels[idx].insert(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tinymap::DefaultKey;

    use super::*;

    #[test]
    fn test_empty() {
        let mut rset = KeySet::<DefaultKey>::new(&KeySetLimits {
            max_level: Level(2),
            num_keys: 4,
        });
        let view = rset.view();
        assert!(!view.levels_remaining());
        view.for_each_level(|_level, _keys, _remaining| {
            unreachable!("no levels should be present");
        });
    }

    #[test]
    fn test_set1() {
        let mut map = tinymap::TinyMap::<DefaultKey, _>::new();
        let key0 = map.insert(0);
        let key1 = map.insert(2);
        let key2 = map.insert(1);
        let key3 = map.insert(1);

        let vals = map.iter().map(|(k, v)| (Level(*v), k));
        let mut rset = KeySet::new(&KeySetLimits {
            max_level: Level(2),
            num_keys: 4,
        });
        rset.extend_above(vals);

        // Test the read path
        {
            let mut expected_level = Level(0);
            rset.view().for_each_level(|level, keys, _remaining| {
                assert_eq!(level, expected_level);
                let expected_keys = match expected_level {
                    Level(0) => vec![key0],
                    Level(1) => vec![key2, key3],
                    Level(2) => vec![key1],
                    _ => vec![],
                };
                itertools::assert_equal(keys, expected_keys);
                expected_level += 1;
            });
        }

        // Test the write path
        {
            let mut expected_level = Level(0);
            rset.view().for_each_level(|level, keys, remaining| {
                assert_eq!(level, expected_level);
                match expected_level {
                    Level(0) => {
                        itertools::assert_equal(keys, vec![key0]);
                        assert!(remaining.is_some());
                        remaining.unwrap().extend_above([(Level(1), key1)]);
                    }
                    Level(1) => {
                        itertools::assert_equal(keys, vec![key1, key2, key3]);
                        assert!(remaining.is_some());
                        remaining.unwrap().extend_above([(Level(2), key2)]);
                    }
                    Level(2) => {
                        itertools::assert_equal(keys, vec![key1, key2]);
                        assert!(remaining.is_none());
                    }
                    _ => unreachable!(),
                }
                expected_level += 1;
            });
        }
    }

    #[test]
    fn test_skip_empty_levels() {
        let limits = KeySetLimits {
            max_level: Level(5),
            num_keys: 10,
        };

        let mut rset = KeySet::new(&limits);

        let key0 = DefaultKey::from(0);
        let key1 = DefaultKey::from(1);
        let key2 = DefaultKey::from(2);

        // Insert keys at non-consecutive levels
        rset.extend_above([(Level(0), key0), (Level(3), key1), (Level(5), key2)]);

        let mut expected_level = Level(0);
        rset.view().for_each_level(|level, keys, _remaining| {
            assert_eq!(level, expected_level);
            match expected_level {
                Level(0) => {
                    itertools::assert_equal(keys, vec![key0]);
                    expected_level = Level(3);
                }
                Level(3) => {
                    itertools::assert_equal(keys, vec![key1]);
                    expected_level = Level(5);
                }
                Level(5) => {
                    itertools::assert_equal(keys, vec![key2]);
                    expected_level = Level(6);
                }
                _ => unreachable!(),
            }
        });
    }
}
