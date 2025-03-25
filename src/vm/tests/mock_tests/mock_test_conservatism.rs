// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=is_mmtk_object

use super::mock_test_prelude::*;

use crate::util::constants::LOG_BITS_IN_WORD;
use crate::util::is_mmtk_object::VO_BIT_REGION_SIZE;
use crate::util::*;

lazy_static! {
    static ref SINGLE_OBJECT: Fixture<SingleObject> = Fixture::new();
}

fn basic_filter(addr: Address) -> bool {
    // `is_mmtk_object` only accept addresses that are aligned to `ObjectReference::ALIGNMENT`.
    // It currently has the same value as `VO_BIT_REGION_SIZE`.
    !addr.is_zero() && addr.is_aligned_to(VO_BIT_REGION_SIZE)
}

fn iter_aligned_offsets(limit: usize) -> impl Iterator<Item = usize> {
    (VO_BIT_REGION_SIZE..limit).step_by(VO_BIT_REGION_SIZE)
}

fn assert_filter_pass(addr: Address) {
    assert!(
        basic_filter(addr),
        "{} should pass basic filter, but failed.",
        addr,
    );
}

fn assert_filter_fail(addr: Address) {
    assert!(
        !basic_filter(addr),
        "{} should fail basic filter, but passed.",
        addr,
    );
}

fn assert_valid_objref(addr: Address) {
    let obj = memory_manager::is_mmtk_object(addr);
    assert!(
        obj.is_some(),
        "mmtk_is_mmtk_object({}) should return Some. Got None.",
        addr,
    );
    assert_eq!(
        obj.unwrap().to_raw_address(),
        addr,
        "mmtk_is_mmtk_object({}) should return Some({}). Got {:?}",
        addr,
        addr,
        obj
    );
}

fn assert_invalid_objref(addr: Address, real: Address) {
    assert!(
        memory_manager::is_mmtk_object(addr).is_none(),
        "mmtk_is_mmtk_object({}) should return None. Got Some. Real object: {}",
        addr,
        real,
    );
}

#[test]
pub fn null() {
    let addr = Address::ZERO;
    // Zero address cannot be passed to `is_mmtk_object`.  We just test if our filter is good.
    assert_filter_fail(addr);
}

// This should be small enough w.r.t `HEAP_START` and `HEAP_END`.
const SMALL_OFFSET: usize = 16384;

#[test]
pub fn too_small() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|fixture| {
                for offset in iter_aligned_offsets(SMALL_OFFSET) {
                    let addr = Address::ZERO + offset;
                    assert_invalid_objref(addr, fixture.objref.to_raw_address());
                }
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn max() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|fixture| {
                let addr = Address::MAX.align_down(VO_BIT_REGION_SIZE);
                assert_invalid_objref(addr, fixture.objref.to_raw_address());
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn too_big() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|fixture| {
                for offset in iter_aligned_offsets(SMALL_OFFSET) {
                    let addr = unsafe { Address::from_usize(0usize.wrapping_sub(offset)) };
                    assert_invalid_objref(addr, fixture.objref.to_raw_address());
                }
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn direct_hit() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|fixture| {
                let addr = fixture.objref.to_raw_address();
                assert_filter_pass(addr);
                assert_valid_objref(addr);
            });
        },
        no_cleanup,
    )
}

const SEVERAL_PAGES: usize = 4 * crate::util::constants::BYTES_IN_PAGE;

#[test]
pub fn small_offsets() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|fixture| {
                for offset in iter_aligned_offsets(SEVERAL_PAGES) {
                    let addr = fixture.objref.to_raw_address() + offset;
                    if basic_filter(addr) {
                        assert_invalid_objref(addr, fixture.objref.to_raw_address());
                    }
                }
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn medium_offsets_aligned() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        let alignment = std::mem::align_of::<Address>();
        for offset in (alignment..(alignment * SEVERAL_PAGES)).step_by(alignment) {
            let addr = fixture.objref.to_raw_address() + offset;
            assert_filter_pass(addr);
            assert_invalid_objref(addr, fixture.objref.to_raw_address());
        }
    });
}

#[test]
pub fn large_offsets_aligned() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|fixture| {
                for log_offset in 12usize..(usize::BITS as usize) {
                    let offset = 1usize << log_offset;
                    let addr = match fixture
                        .objref
                        .to_raw_address()
                        .as_usize()
                        .checked_add(offset)
                    {
                        Some(n) => unsafe { Address::from_usize(n) },
                        None => break,
                    };
                    assert_filter_pass(addr);
                    assert_invalid_objref(addr, fixture.objref.to_raw_address());
                }
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn negative_offsets() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|fixture| {
                for log_offset in LOG_BITS_IN_WORD..(usize::BITS as usize) {
                    let offset = 1usize << log_offset;
                    let addr = match fixture
                        .objref
                        .to_raw_address()
                        .as_usize()
                        .checked_sub(offset)
                    {
                        Some(0) => break,
                        Some(n) => unsafe { Address::from_usize(n) },
                        None => break,
                    };
                    assert_filter_pass(addr);
                    assert_invalid_objref(addr, fixture.objref.to_raw_address());
                }
            });
        },
        no_cleanup,
    )
}
