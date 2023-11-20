//! This module provides an iterator that groups adjacent items with the same key.
//!
//! It gives all `Iterator + Clone` iterators a new method: `.revisitable_group_by`. It is similar
//! to `Itertools::group_by`, but it lets the user know the length of each group before iterating
//! through the group.  Implementation-wise, it eagerly finds all items with the same key, and
//! then lets the user traverse the same range of items again using a pre-cloned iterator. This
//! is why it is named "revisitable" group-by.
//!
//! This is useful for the memory mapper to coalesce the `mmap` call for adjacent chunks that have
//! the same `MapState`.  The memory mapper needs to know the size of each group to compute the
//! memory range the group of `MapState` covers in order to call `mmap`, and then traverse the
//! group of `MapState` again to update them.
//!
//! The `.revisitable_group_by` method takes a closure for computing the keys of each item.
//! Adjacent items with the same key will be put into the same group.
//!
//! The following example groups adjacent even or odd numbers together.
//!
//! ```rs
//! let nums = [1, 3, 5, 2, 4, 6, 7, 9];
//! for group in nums.iter().revisitable_group_by(|x| *x % 2) {
//!     println!("key: {}, len: {}", group.key, group.len);
//!     for x in group {
//!         println!("  x: {}", *x);
//!     }
//! }
//! ```
//!
//! It should form three groups, `[1, 3, 5]`, `[2, 4, 6]` and `[7, 9]`, with the keys being 1, 0
//! and 1, respectively.
//!
//! It can be used with the `.flatten()` method to make groups across the boundaries of several
//! iterable items.
//!
//! ```rs
//! let slice_of_slices: &[&[i32]] = &[&[10, 20], &[30, 40, 11, 21], &[31, 12, 22]];
//! let result = slice_of_slices.iter().copied().flatten().copied()
//!     .revisitable_group_by(|x| x % 10)
//!     .map(|group| group.collect::<Vec<_>>())
//!     .collect::<Vec<_>>();
//! assert_eq!(
//!     result,
//!     vec![vec![10, 20, 30, 40], vec![11, 21, 31], vec![12, 22]],
//! );
//! ```

/// This trait provides the `revisitable_group_by` method for all `Iterator` that also implements
/// `Clone`.
pub(crate) trait RevisitableGroupByForIterator {
    type Item;
    type Iter: Iterator<Item = Self::Item> + Clone;

    /// Group adjacent items by key.  `get_key` is a closure that computes the key.
    fn revisitable_group_by<K, F>(
        self,
        get_key: F,
    ) -> RevisitableGroupBy<Self::Item, K, Self::Iter, F>
    where
        K: PartialEq + Copy,
        F: FnMut(&Self::Item) -> K;
}

impl<I: Iterator + Clone> RevisitableGroupByForIterator for I {
    type Item = <I as Iterator>::Item;
    type Iter = I;

    fn revisitable_group_by<K, F>(
        self,
        get_key: F,
    ) -> RevisitableGroupBy<Self::Item, K, Self::Iter, F>
    where
        K: PartialEq + Copy,
        F: FnMut(&Self::Item) -> K,
    {
        RevisitableGroupBy {
            iter: self,
            get_key,
            next_group_initial: None,
        }
    }
}

/// An iterator through groups of items with the same key.
pub(crate) struct RevisitableGroupBy<T, K, I, F>
where
    K: PartialEq + Copy,
    I: Iterator<Item = T> + Clone,
    F: FnMut(&T) -> K,
{
    /// The underlying iterator.
    iter: I,
    /// The function to get the key.
    get_key: F,
    /// Temporarily save the item and key of the next group when peeking.
    next_group_initial: Option<(T, K)>,
}

impl<T, K, I, F> Iterator for RevisitableGroupBy<T, K, I, F>
where
    K: PartialEq + Copy,
    I: Iterator<Item = T> + Clone,
    F: FnMut(&T) -> K,
{
    type Item = RevisitableGroup<T, K, I>;

    fn next(&mut self) -> Option<Self::Item> {
        let (group_head, group_key) = if let Some((head, key)) = self.next_group_initial.take() {
            // We already peeked the item of the next group the last time `next()` was called.
            // Count that in.
            (head, key)
        } else {
            // Either we haven't start iterating, yet, or we already exhausted the iter.
            // Get the next item from the underlying iter.
            if let Some(item) = self.iter.next() {
                // The next group has at least one item.
                // This is the key of the group.
                let key = (self.get_key)(&item);
                (item, key)
            } else {
                return None;
            }
        };

        // If reached here, the group must have at least one item.
        let mut group_size = 1;

        // Get the rest of the group.
        let saved_iter = self.iter.clone();
        loop {
            if let Some(item) = self.iter.next() {
                // The next item exists. It either belongs to the current group or not.
                let key = (self.get_key)(&item);
                if key == group_key {
                    // It is in the same group.
                    group_size += 1;
                } else {
                    // It belongs to the next group.  Save the item and the key...
                    self.next_group_initial = Some((item, key));
                    // ... and we have a group now.
                    break;
                }
            } else {
                // No more items. This is the last group.
                debug_assert!(self.next_group_initial.is_none());
                break;
            }
        }

        Some(RevisitableGroup {
            key: group_key,
            len: group_size,
            head: Some(group_head),
            iter: saved_iter,
            remaining: group_size,
        })
    }
}

pub(crate) struct RevisitableGroup<T, K, I>
where
    K: PartialEq + Copy,
    I: Iterator<Item = T>,
{
    /// The key of this group.
    pub key: K,
    /// The length of this group.
    pub len: usize,
    /// The first item. Note that `iter` starts from the second element due to the way we clone it.
    head: Option<T>,
    /// The underlying iterator.
    iter: I,
    /// The number of items remain to be iterated.
    remaining: usize,
}

impl<T, K, I> Iterator for RevisitableGroup<T, K, I>
where
    K: PartialEq + Copy,
    I: Iterator<Item = T>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            None
        } else {
            self.remaining -= 1;
            if let Some(item) = self.head.take() {
                Some(item)
            } else {
                let result = self.iter.next();
                debug_assert!(result.is_some());
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_group_by() {
        let nums = [1, 3, 5, 2, 4, 6, 7, 9];
        let grouped = nums
            .iter()
            .revisitable_group_by(|x| *x % 2)
            .map(|group| (group.key, group.len, group.copied().collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        assert_eq!(
            grouped,
            vec![
                (1, 3, vec![1, 3, 5]),
                (0, 3, vec![2, 4, 6]),
                (1, 2, vec![7, 9]),
            ]
        );
    }

    #[test]
    #[allow(clippy::never_loop)] // We are testing with empty slices. The panic in the loop body should not run.
    fn test_empty_outer_slice() {
        let slice_of_slices: &[&[i32]] = &[];
        for _group in slice_of_slices
            .iter()
            .copied()
            .flatten()
            .copied()
            .revisitable_group_by(|_| 42)
        {
            panic!("There is no item!");
        }
    }

    #[test]
    #[allow(clippy::never_loop)] // We are testing with empty slices. The panic in the loop body should not run.
    fn test_empty_inner_slice() {
        let slice_of_slices: &[&[i32]] = &[&[], &[], &[]];
        for _group in slice_of_slices
            .iter()
            .copied()
            .flatten()
            .copied()
            .revisitable_group_by(|_| 42)
        {
            panic!("There is no item!");
        }
    }

    #[test]
    fn test_single_item() {
        let slice_of_slices: &[&[i32]] = &[&[1]];
        for group in slice_of_slices
            .iter()
            .copied()
            .flatten()
            .copied()
            .revisitable_group_by(|_| 42)
        {
            assert_eq!(group.key, 42);
        }
    }

    #[test]
    fn test_single_slice_multi_item() {
        let slice_of_slices: &[&[i32]] = &[&[1, 3, 5, 2, 4, 6, 7]];
        let result = slice_of_slices
            .iter()
            .copied()
            .flatten()
            .copied()
            .revisitable_group_by(|x| x % 2)
            .map(|group| (group.key, group.len, group.collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        assert_eq!(
            result,
            vec![
                (1, 3, vec![1, 3, 5]),
                (0, 3, vec![2, 4, 6]),
                (1, 1, vec![7])
            ]
        );
    }

    #[test]
    fn test_multi_slice_multi_item() {
        let slice_of_slices: &[&[i32]] = &[&[10, 20], &[11, 21, 31], &[12, 22, 32, 42]];
        let result = slice_of_slices
            .iter()
            .copied()
            .flatten()
            .copied()
            .revisitable_group_by(|x| x % 10)
            .map(|group| (group.key, group.len, group.collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        assert_eq!(
            result,
            vec![
                (0, 2, vec![10, 20]),
                (1, 3, vec![11, 21, 31]),
                (2, 4, vec![12, 22, 32, 42])
            ]
        );
    }

    #[test]
    fn test_cross_slice_groups() {
        let slice_of_slices: &[&[i32]] = &[&[10, 20], &[30, 40, 11, 21], &[31, 12, 22]];
        let result = slice_of_slices
            .iter()
            .copied()
            .flatten()
            .copied()
            .revisitable_group_by(|x| x % 10)
            .map(|group| (group.key, group.len, group.collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        assert_eq!(
            result,
            vec![
                (0, 4, vec![10, 20, 30, 40]),
                (1, 3, vec![11, 21, 31]),
                (2, 2, vec![12, 22])
            ]
        );
    }

    #[test]
    fn test_cross_slice_groups2() {
        let slice_of_slices: &[&[i32]] = &[&[10, 20, 11], &[21, 31, 41], &[51, 61], &[71, 12, 22]];
        let result = slice_of_slices
            .iter()
            .cloned()
            .flatten()
            .copied()
            .revisitable_group_by(|x| x % 10)
            .map(|group| (group.key, group.len, group.collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        assert_eq!(
            result,
            vec![
                (0, 2, vec![10, 20]),
                (1, 7, vec![11, 21, 31, 41, 51, 61, 71]),
                (2, 2, vec![12, 22])
            ]
        );
    }

    #[test]
    fn test_internal_mutability() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let slab0 = vec![
            AtomicUsize::new(1),
            AtomicUsize::new(3),
            AtomicUsize::new(2),
        ];
        let slab1 = vec![
            AtomicUsize::new(4),
            AtomicUsize::new(6),
            AtomicUsize::new(5),
        ];
        let slab2 = vec![
            AtomicUsize::new(7),
            AtomicUsize::new(9),
            AtomicUsize::new(10),
        ];

        // Note: We only take the first two elements from slab2,
        // because the mmapper sometimes processes part of a slab.
        let slices: Vec<&[AtomicUsize]> = vec![&slab0[0..3], &slab1[0..3], &slab2[0..2]];

        let mut collected = vec![];

        for group in slices
            .iter()
            .copied()
            .flatten()
            .revisitable_group_by(|x| x.load(Ordering::SeqCst) % 2)
        {
            let mut group_collected = vec![];
            let key = group.key;
            for elem in group {
                let value = elem.load(Ordering::SeqCst);
                group_collected.push(value);

                let new_value = value * 100 + key;
                elem.store(new_value, Ordering::SeqCst);
            }

            collected.push(group_collected);
        }

        assert_eq!(collected, vec![vec![1, 3], vec![2, 4, 6], vec![5, 7, 9]]);

        let load_all = |slab: Vec<AtomicUsize>| {
            slab.iter()
                .map(|x| x.load(Ordering::SeqCst))
                .collect::<Vec<_>>()
        };

        assert_eq!(load_all(slab0), vec![101, 301, 200]);
        assert_eq!(load_all(slab1), vec![400, 600, 501]);
        assert_eq!(load_all(slab2), vec![701, 901, 10]); // The last item should not be affected.
    }
}
