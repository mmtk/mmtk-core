// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=vo_map

use crate::api::*;
use crate::object_model::OBJECT_REF_OFFSET;
use crate::tests::fixtures::{Fixture, SingleObject};
use mmtk::util::constants::LOG_BITS_IN_WORD;
use mmtk::util::metadata::vo_bit::VO_BIT_REGION_SIZE;
use mmtk::util::*;

lazy_static! {
    static ref SINGLE_OBJECT: Fixture<SingleObject> = Fixture::new();
}

fn basic_filter(addr: Address) -> bool {
    !addr.is_zero() && addr.as_usize() % VO_BIT_REGION_SIZE == (OBJECT_REF_OFFSET % VO_BIT_REGION_SIZE)
}

fn assert_filter_pass(object: ObjectReference) {
    assert!(
        basic_filter(object.to_address()),
        "{} should pass basic filter, but failed.",
        object,
    );
}

fn assert_filter_fail(object: ObjectReference) {
    assert!(
        !basic_filter(object.to_address()),
        "{} should fail basic filter, but passed.",
        object,
    );
}

fn assert_valid_objref(object: ObjectReference) {
    assert!(
        mmtk_is_valid_mmtk_object(object),
        "mmtk_is_valid_mmtk_object({}) should return true. Got false.",
        object,
    );
}

fn assert_invalid_objref(object: ObjectReference, real: ObjectReference) {
    assert!(
        !mmtk_is_valid_mmtk_object(object),
        "mmtk_is_valid_mmtk_object({}) should return false. Got true. Real object: {}",
        object,
        real,
    );
}

#[test]
pub fn null() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        let object = ObjectReference::NULL;
        assert_filter_fail(object);
        assert_invalid_objref(object, fixture.objref);
    });
}

// This should be small enough w.r.t `HEAP_START` and `HEAP_END`.
const SMALL_OFFSET: usize = 16384;

#[test]
pub fn too_small() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        for offset in 1usize..SMALL_OFFSET {
            let addr = Address::ZERO + offset;
            assert_invalid_objref(unsafe { addr.to_object_reference() }, fixture.objref);
        }
    });
}

#[test]
pub fn max() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        let addr = Address::MAX;
        let object = unsafe { addr.to_object_reference() };
        assert_invalid_objref(object, fixture.objref);
    });
}

#[test]
pub fn too_big() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        for offset in 1usize..SMALL_OFFSET {
            let addr = Address::MAX - offset;
            let object = unsafe { addr.to_object_reference() };
            assert_invalid_objref(object, fixture.objref);
        }
    });
}

#[test]
pub fn direct_hit() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        let object = fixture.objref;
        assert_filter_pass(object);
        assert_valid_objref(object);
    });
}

const SEVERAL_PAGES: usize = 4 * mmtk::util::constants::BYTES_IN_PAGE;

#[test]
pub fn small_offsets() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        for offset in 1usize..SEVERAL_PAGES {
            let addr = fixture.objref.to_address() + offset;
            if basic_filter(addr) {
                let object = unsafe { addr.to_object_reference() };
                assert_invalid_objref(object, fixture.objref);
            }
        }
    });
}

#[test]
pub fn medium_offsets_aligned() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        let alignment = std::mem::align_of::<Address>();
        for offset in (alignment..(alignment * SEVERAL_PAGES)).step_by(alignment) {
            let addr = fixture.objref.to_address() + offset;
            let object = unsafe { addr.to_object_reference() };
            assert_filter_pass(object);
            assert_invalid_objref(object, fixture.objref);
        }
    });
}

#[test]
pub fn large_offsets_aligned() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        for log_offset in 12usize..(usize::BITS as usize) {
            let offset = 1usize << log_offset;
            let addr = match fixture.objref.to_address().as_usize().checked_add(offset) {
                Some(n) => unsafe { Address::from_usize(n) },
                None => break,
            };
            let object = unsafe { addr.to_object_reference() };
            assert_filter_pass(object);
            assert_invalid_objref(object, fixture.objref);
        }
    });
}

#[test]
pub fn negative_offsets() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        for log_offset in LOG_BITS_IN_WORD..(usize::BITS as usize) {
            let offset = 1usize << log_offset;
            let addr = match fixture.objref.to_address().as_usize().checked_sub(offset) {
                Some(0) => break,
                Some(n) => unsafe { Address::from_usize(n) },
                None => break,
            };
            let object = unsafe { addr.to_object_reference() };
            assert_filter_pass(object);
            assert_invalid_objref(object, fixture.objref);
        }
    });
}
