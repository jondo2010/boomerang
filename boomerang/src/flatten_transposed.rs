//! # Flatten Transposed Iterator
//!
//! This module provides a `FlattenTransposed` iterator adapter that flattens nested iterables while transposing the iteration order.
//!
//! ## Features
//!
//! - Flattens nested iterables (e.g., `Vec<Vec<T>>` to `Vec<T>`)
//! - Transposes the iteration order (iterates "vertically" through nested structures)
//!
//! ## Example
//!
//! ```rust
//! use boomerang::flatten_transposed::FlattenTransposedExt;
//!
//! let nested = [
//!     vec![1, 2, 3],
//!     vec![4, 5],
//!     vec![6, 7, 8, 9]
//! ];
//!
//! let flattened: Vec<_> = nested
//!     .into_iter()
//!     .flatten_transposed()
//!     .collect();
//!
//! assert_eq!(flattened, vec![1, 4, 6, 2, 5, 7, 3, 8, 9]);
//! ```
//!
//! In this example, the `flatten_transposed()` method is called on an iterator
//! of vectors. The resulting iterator yields elements in a transposed order,
//! effectively "zipping" the inner vectors together and then flattening the result.

use std::iter::FusedIterator;

pub struct FlattenTransposed<I>
where
    I: Iterator,
    I::Item: IntoIterator,
{
    inner_iters: Vec<<I::Item as IntoIterator>::IntoIter>,
    current_index: usize,
}

impl<I> FlattenTransposed<I>
where
    I: Iterator,
    I::Item: IntoIterator,
{
    fn new(iter: I) -> Self {
        let inner_iters = iter.map(IntoIterator::into_iter).collect();
        FlattenTransposed {
            inner_iters,
            current_index: 0,
        }
    }
}

impl<I> Iterator for FlattenTransposed<I>
where
    I: Iterator,
    I::Item: IntoIterator,
{
    type Item = <I::Item as IntoIterator>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.inner_iters.is_empty() {
            if let Some(item) = self.inner_iters[self.current_index].next() {
                self.current_index = (self.current_index + 1) % self.inner_iters.len();
                return Some(item);
            } else {
                let _ = self.inner_iters.remove(self.current_index);
                if !self.inner_iters.is_empty() {
                    self.current_index %= self.inner_iters.len();
                }
            }
        }

        None
    }
}

impl<I> FusedIterator for FlattenTransposed<I>
where
    I: Iterator + FusedIterator,
    I::Item: IntoIterator,
    <I::Item as IntoIterator>::IntoIter: FusedIterator,
{
}

pub trait FlattenTransposedExt: Iterator {
    fn flatten_transposed(self) -> FlattenTransposed<Self>
    where
        Self: Sized,
        Self::Item: IntoIterator,
    {
        FlattenTransposed::new(self)
    }
}

impl<T: ?Sized> FlattenTransposedExt for T where T: Iterator {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatten_transposed() {
        let xy = [[1, 2, 3], [4, 5, 6]];
        let flat_xposed: Vec<_> = xy.iter().flatten_transposed().copied().collect();
        assert_eq!(flat_xposed, vec![1, 4, 2, 5, 3, 6]);
    }

    #[test]
    fn test_uneven_iterators() {
        let xy = [vec![1, 2, 3], vec![4, 5], vec![6, 7, 8, 9]];
        let flat_xposed: Vec<_> = xy.iter().flatten_transposed().copied().collect();
        assert_eq!(flat_xposed, vec![1, 4, 6, 2, 5, 7, 3, 8, 9]);
    }
}
