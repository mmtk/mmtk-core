//! This module is a tool for visiting slice of slices and group adjacent elements by keys.
//! It is useful for coalescing `mmap` calls for requests from `FragmentedMapper`.

/// A group of adjacent elements in a slice of slices with the same key.
pub(crate) struct Group<'s, T, K>
where
    K: PartialEq + Copy,
{
    slice_of_slices: &'s [&'s [T]],
    /// The key of the elements in this group
    pub(crate) key: K,
    start_outer: usize,
    start_inner: usize,
    /// The starting index of the group among all elements in the slice of slices.
    pub(crate) start_total: usize,
    end_outer: usize,
    end_inner: usize,
    /// The ending index (exclusive) of the group among all elements in the slice of slices.
    pub(crate) end_total: usize,
}

impl<'s, T, K> Group<'s, T, K>
where
    K: PartialEq + Copy,
{
    pub(crate) fn iter(&'s self) -> GroupIterator<'s, T, K> {
        GroupIterator {
            group: self,
            cur_outer: self.start_outer,
            cur_inner: self.start_inner,
        }
    }
}

/// Iterate elements in a group.
pub(crate) struct GroupIterator<'g, T, K>
where
    K: PartialEq + Copy,
{
    group: &'g Group<'g, T, K>,
    cur_outer: usize,
    cur_inner: usize,
}

impl<'g, T, K> Iterator for GroupIterator<'g, T, K>
where
    K: PartialEq + Copy,
{
    type Item = &'g T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_outer == self.group.end_outer && self.cur_inner == self.group.end_inner {
            return None;
        }

        let rv = &self.group.slice_of_slices[self.cur_outer][self.cur_inner];
        self.cur_inner += 1;
        if self.cur_inner == self.group.slice_of_slices[self.cur_outer].len() {
            self.cur_inner = 0;
            self.cur_outer += 1;
        }

        Some(rv)
    }
}

/// An iterator that interates through groups of elements in a slice of slice.
///
/// Each group contains a contiguous range of elements that may span multiple inner slices.
/// All elements in a group share the same key.
pub(crate) struct SliceOfSlicesGrouper<'s, T, K, F>
where
    K: PartialEq + Copy,
    F: FnMut(&T) -> K,
{
    slice_of_slices: &'s [&'s [T]],
    get_key: F,
    start_outer: usize,
    start_inner: usize,
    start_total: usize,
    end_outer: usize,
    end_inner: usize,
    end_total: usize,
    maybe_prev_key: Option<K>,
    done: bool,
}

impl<'s, T, K, F> SliceOfSlicesGrouper<'s, T, K, F>
where
    K: PartialEq + Copy,
    F: FnMut(&T) -> K,
{
    /// Create a `SliceOfSlicesGrouper`.
    ///
    /// -   `slice_of_slices: the slice of slices to iterate through.
    /// -   `get_key`: a closure that maps each element to a key.
    pub fn new(slice_of_slices: &'s [&'s [T]], get_key: F) -> Self {
        Self {
            slice_of_slices,
            get_key,
            start_outer: 0,
            start_inner: 0,
            start_total: 0,
            end_outer: 0,
            end_inner: 0,
            end_total: 0,
            maybe_prev_key: None,
            done: false,
        }
    }

    fn inspect_current_element(&mut self) -> (K, Option<Group<'s, T, K>>) {
        debug_assert!(
            !self.slice_of_slices[self.end_outer].is_empty(),
            "An inner slice is empty"
        );

        let cur_key = (self.get_key)(&self.slice_of_slices[self.end_outer][self.end_inner]);

        let maybe_group = match self.maybe_prev_key {
            Some(prev_key) if prev_key != cur_key => Some(self.capture()),
            _ => None,
        };

        (cur_key, maybe_group)
    }

    fn capture(&self) -> Group<'s, T, K> {
        debug_assert!(self.maybe_prev_key.is_some());

        Group {
            slice_of_slices: self.slice_of_slices,
            key: self.maybe_prev_key.unwrap(),
            start_outer: self.start_outer,
            start_inner: self.start_inner,
            start_total: self.start_total,
            end_outer: self.end_outer,
            end_inner: self.end_inner,
            end_total: self.end_total,
        }
    }

    fn has_next(&self) -> bool {
        self.end_outer < self.slice_of_slices.len()
    }

    fn reset_start(&mut self) {
        self.start_outer = self.end_outer;
        self.start_inner = self.end_inner;
        self.start_total = self.end_total;
    }

    fn proceed(&mut self) {
        self.end_total += 1;
        self.end_inner += 1;
        if self.end_inner == self.slice_of_slices[self.end_outer].len() {
            self.end_inner = 0;
            self.end_outer += 1;
        }
    }
}

impl<'s, T, K, F> Iterator for SliceOfSlicesGrouper<'s, T, K, F>
where
    K: PartialEq + Copy,
    F: FnMut(&T) -> K,
{
    type Item = Group<'s, T, K>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.has_next() {
            let (cur_key, maybe_group) = self.inspect_current_element();
            self.maybe_prev_key = Some(cur_key);
            if maybe_group.is_some() {
                self.reset_start();
            }
            self.proceed();

            if maybe_group.is_some() {
                return maybe_group;
            }
        }

        if self.maybe_prev_key.is_some() {
            debug_assert_eq!(self.end_outer, self.slice_of_slices.len());
            debug_assert_eq!(self.end_inner, 0);

            let group = self.capture();
            self.maybe_prev_key = None;

            Some(group)
        } else {
            None
        }
    }
}

#[test]
fn test_empty_outer_slice() {
    let slice_of_slices: &[&[i32]] = &[];
    for _group in SliceOfSlicesGrouper::new(slice_of_slices, |_| 42) {
        panic!("There is no element!");
    }
}

// Note: Empty inner slices are not allowed.

#[test]
fn test_single_element() {
    let slice_of_slices: &[&[i32]] = &[&[1]];
    for group in SliceOfSlicesGrouper::new(slice_of_slices, |_| 42) {
        assert_eq!(group.key, 42);
    }
}

#[test]
fn test_single_slice_multi_element() {
    let slice_of_slices: &[&[i32]] = &[&[1, 3, 5, 2, 4, 6, 7]];
    let result = SliceOfSlicesGrouper::new(slice_of_slices, |x| x % 2)
        .map(|group| (group.key, group.iter().copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    assert_eq!(
        result,
        vec![(1, vec![1, 3, 5]), (0, vec![2, 4, 6]), (1, vec![7])]
    );
}

#[test]
fn test_multi_slice_multi_element() {
    let slice_of_slices: &[&[i32]] = &[&[10, 20], &[11, 21, 31], &[12, 22, 32, 42]];
    let result = SliceOfSlicesGrouper::new(slice_of_slices, |x| x % 10)
        .map(|group| (group.key, group.iter().copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    assert_eq!(
        result,
        vec![
            (0, vec![10, 20]),
            (1, vec![11, 21, 31]),
            (2, vec![12, 22, 32, 42])
        ]
    );
}

#[test]
fn test_cross_slice_groups() {
    let slice_of_slices: &[&[i32]] = &[&[10, 20], &[30, 40, 11, 21], &[31, 12, 22]];
    let result = SliceOfSlicesGrouper::new(slice_of_slices, |x| x % 10)
        .map(|group| (group.key, group.iter().copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    assert_eq!(
        result,
        vec![
            (0, vec![10, 20, 30, 40]),
            (1, vec![11, 21, 31]),
            (2, vec![12, 22])
        ]
    );
}

#[test]
fn test_cross_slice_groups2() {
    let slice_of_slices: &[&[i32]] = &[&[10, 20, 11], &[21, 31, 41], &[51, 61], &[71, 12, 22]];
    let result = SliceOfSlicesGrouper::new(slice_of_slices, |x| x % 10)
        .map(|group| (group.key, group.iter().copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    assert_eq!(
        result,
        vec![
            (0, vec![10, 20]),
            (1, vec![11, 21, 31, 41, 51, 61, 71]),
            (2, vec![12, 22])
        ]
    );
}
