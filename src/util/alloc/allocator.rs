use ::util::address::Address;

use ::policy::space::Space;

#[inline(always)]
pub fn align_allocation(region: Address, align: usize, offset: isize) -> Address {
    let region_isize = region.as_usize() as isize;

    let mask = (align - 1) as isize; // fromIntSignExtend
    let neg_off = -offset; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    region + delta
}

pub trait Allocator<'a, T> where T: Space {
    fn new(thread_id: usize, space: &'a T) -> Self;

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address;
}