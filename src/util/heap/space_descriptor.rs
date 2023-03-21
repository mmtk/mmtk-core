use crate::util::constants::*;
use crate::util::heap::layout::vm_layout_constants;
use crate::util::Address;
use std::sync::atomic::{AtomicUsize, Ordering};

const TYPE_BITS: usize = 2;
#[allow(unused)]
const TYPE_SHARED: usize = 0;
const TYPE_CONTIGUOUS: usize = 1;
const TYPE_CONTIGUOUS_HI: usize = 3;
const TYPE_MASK: usize = (1 << TYPE_BITS) - 1;
const SIZE_SHIFT: usize = TYPE_BITS;
const SIZE_BITS: usize = 10;
#[cfg(target_pointer_width = "32")]
const SIZE_MASK: usize = ((1 << SIZE_BITS) - 1) << SIZE_SHIFT;
const EXPONENT_SHIFT: usize = SIZE_SHIFT + SIZE_BITS;
const EXPONENT_BITS: usize = 5;
#[cfg(target_pointer_width = "32")]
const EXPONENT_MASK: usize = ((1 << EXPONENT_BITS) - 1) << EXPONENT_SHIFT;
const MANTISSA_SHIFT: usize = EXPONENT_SHIFT + EXPONENT_BITS;
const MANTISSA_BITS: usize = 14;
const BASE_EXPONENT: usize = BITS_IN_INT - MANTISSA_BITS;

// get_index() is only implemented for 64 bits
#[cfg(target_pointer_width = "64")]
const INDEX_MASK: usize = !TYPE_MASK;
const INDEX_SHIFT: usize = TYPE_BITS;

static DISCONTIGUOUS_SPACE_INDEX: AtomicUsize = AtomicUsize::new(DISCONTIG_INDEX_INCREMENT);
const DISCONTIG_INDEX_INCREMENT: usize = 1 << TYPE_BITS;

#[derive(Copy, Clone, PartialEq, Debug)]
pub struct SpaceDescriptor(usize);

impl SpaceDescriptor {
    pub const UNINITIALIZED: Self = SpaceDescriptor(0);

    pub fn create_descriptor_from_heap_range(start: Address, end: Address) -> SpaceDescriptor {
        let top = end == vm_layout_constants::HEAP_END;
        if cfg!(target_pointer_width = "64") {
            let space_index = if start > vm_layout_constants::HEAP_END {
                ::std::usize::MAX
            } else {
                start >> vm_layout_constants::SPACE_SHIFT_64
            };
            return SpaceDescriptor(
                space_index << INDEX_SHIFT
                    | (if top {
                        TYPE_CONTIGUOUS_HI
                    } else {
                        TYPE_CONTIGUOUS
                    }),
            );
        }
        let chunks = (end - start) >> vm_layout_constants::LOG_BYTES_IN_CHUNK;
        debug_assert!(!start.is_zero() && chunks > 0 && chunks < (1 << SIZE_BITS));
        let mut tmp = start >> BASE_EXPONENT;
        let mut exponent = 0;
        while (tmp != 0) && ((tmp & 1) == 0) {
            tmp >>= 1;
            exponent += 1;
        }
        let mantissa = tmp;
        debug_assert!((tmp << (BASE_EXPONENT + exponent)) == start.as_usize());
        SpaceDescriptor(
            (mantissa << MANTISSA_SHIFT)
                | (exponent << EXPONENT_SHIFT)
                | (chunks << SIZE_SHIFT)
                | (if top {
                    TYPE_CONTIGUOUS_HI
                } else {
                    TYPE_CONTIGUOUS
                }),
        )
    }

    pub fn create_descriptor() -> SpaceDescriptor {
        let next =
            DISCONTIGUOUS_SPACE_INDEX.fetch_add(DISCONTIG_INDEX_INCREMENT, Ordering::Relaxed);
        let ret = SpaceDescriptor(next);
        debug_assert!(!ret.is_contiguous());
        ret
    }

    pub fn is_empty(self) -> bool {
        self.0 == SpaceDescriptor::UNINITIALIZED.0
    }

    pub fn is_contiguous(self) -> bool {
        (self.0 & TYPE_CONTIGUOUS) == TYPE_CONTIGUOUS
    }

    pub fn is_contiguous_hi(self) -> bool {
        (self.0 & TYPE_MASK) == TYPE_CONTIGUOUS_HI
    }

    #[cfg(target_pointer_width = "64")]
    pub fn get_start(self) -> Address {
        use crate::util::heap::layout::heap_parameters;
        unsafe { Address::from_usize(self.get_index() << heap_parameters::LOG_SPACE_SIZE_64) }
    }

    #[cfg(target_pointer_width = "32")]
    pub fn get_start(self) -> Address {
        debug_assert!(self.is_contiguous());

        let descriptor = self.0;
        let mantissa = descriptor >> MANTISSA_SHIFT;
        let exponent = (descriptor & EXPONENT_MASK) >> EXPONENT_SHIFT;
        unsafe { Address::from_usize(mantissa << (BASE_EXPONENT + exponent)) }
    }

    #[cfg(target_pointer_width = "64")]
    pub fn get_extent(self) -> usize {
        vm_layout_constants::SPACE_SIZE_64
    }

    #[cfg(target_pointer_width = "32")]
    pub fn get_extent(self) -> usize {
        debug_assert!(self.is_contiguous());
        let chunks = (self.0 & SIZE_MASK) >> SIZE_SHIFT;
        chunks << vm_layout_constants::LOG_BYTES_IN_CHUNK
    }

    pub fn get_index(self) -> usize {
        debug_assert!(cfg!(target_pointer_width = "64"));
        (self.0 & INDEX_MASK) >> INDEX_SHIFT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::heap::layout::vm_layout_constants::*;

    #[test]
    fn create_discontiguous_descriptor() {
        let d1 = SpaceDescriptor::create_descriptor();
        assert!(!d1.is_empty());
        assert!(!d1.is_contiguous());
        assert!(!d1.is_contiguous_hi());

        let d2 = SpaceDescriptor::create_descriptor();
        assert!(!d2.is_empty());
        assert!(!d2.is_contiguous());
        assert!(!d2.is_contiguous_hi());
    }

    const TEST_SPACE_SIZE: usize = BYTES_IN_CHUNK * 10;

    #[test]
    fn create_contiguous_descriptor_at_heap_start() {
        let d = SpaceDescriptor::create_descriptor_from_heap_range(
            HEAP_START,
            HEAP_START + TEST_SPACE_SIZE,
        );
        assert!(!d.is_empty());
        assert!(d.is_contiguous());
        assert!(!d.is_contiguous_hi());
        assert_eq!(d.get_start(), HEAP_START);
        if cfg!(target_pointer_width = "64") {
            assert_eq!(d.get_extent(), SPACE_SIZE_64);
        } else {
            assert_eq!(d.get_extent(), TEST_SPACE_SIZE);
        }
    }

    #[test]
    fn create_contiguous_descriptor_in_heap() {
        let d = SpaceDescriptor::create_descriptor_from_heap_range(
            HEAP_START + TEST_SPACE_SIZE,
            HEAP_START + TEST_SPACE_SIZE * 2,
        );
        assert!(!d.is_empty());
        assert!(d.is_contiguous());
        assert!(!d.is_contiguous_hi());
        if cfg!(target_pointer_width = "64") {
            assert_eq!(d.get_start(), HEAP_START);
            assert_eq!(d.get_extent(), SPACE_SIZE_64);
        } else {
            assert_eq!(d.get_start(), HEAP_START + TEST_SPACE_SIZE);
            assert_eq!(d.get_extent(), TEST_SPACE_SIZE);
        }
    }

    #[test]
    fn create_contiguous_descriptor_at_heap_end() {
        let d = SpaceDescriptor::create_descriptor_from_heap_range(
            HEAP_END - TEST_SPACE_SIZE,
            HEAP_END,
        );
        assert!(!d.is_empty());
        assert!(d.is_contiguous());
        assert!(d.is_contiguous_hi());
        if cfg!(target_pointer_width = "64") {
            assert_eq!(d.get_start(), HEAP_END - SPACE_SIZE_64);
            assert_eq!(d.get_extent(), SPACE_SIZE_64);
        } else {
            assert_eq!(d.get_start(), HEAP_END - TEST_SPACE_SIZE);
            assert_eq!(d.get_extent(), TEST_SPACE_SIZE);
        }
    }
}
