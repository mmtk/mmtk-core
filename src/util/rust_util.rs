/// Const funciton for min value of two usize numbers.
pub const fn min_of_usize(a: usize, b: usize) -> usize {
    [a, b][(a < b) as usize]
}
