// GITHUB-CI: MMTK_PLAN=all

use crate::api::mmtk_is_in_mmtk_spaces as is_in_mmtk_spaces;
use crate::test_fixtures::{Fixture, SingleObject};
use mmtk::util::*;

lazy_static! {
    static ref SINGLE_OBJECT: Fixture<SingleObject> = Fixture::new();
}

#[test]
pub fn null() {
    SINGLE_OBJECT.with_fixture(|_fixture| {
        assert!(
            !is_in_mmtk_spaces(ObjectReference::NULL),
            "NULL pointer should not be in any MMTk spaces."
        );
    });
}

#[test]
pub fn max() {
    SINGLE_OBJECT.with_fixture(|_fixture| {
        assert!(
            !is_in_mmtk_spaces(ObjectReference::from_raw_address(Address::MAX)),
            "Address::MAX should not be in any MMTk spaces."
        );
    });
}

#[test]
pub fn direct_hit() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        assert!(
            is_in_mmtk_spaces(fixture.objref),
            "The address of the allocated object should be in the space"
        );
    });
}

#[test]
pub fn large_offsets_aligned() {
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
            // It's just a smoke test.  It is hard to predict if the addr is still in any space,
            // but it must not crash.
            let _ = is_in_mmtk_spaces(ObjectReference::from_raw_address(addr));
        }
    });
}

#[test]
pub fn negative_offsets() {
    SINGLE_OBJECT.with_fixture(|fixture| {
        for log_offset in 1usize..(usize::BITS as usize) {
            let offset = 1usize << log_offset;
            let addr = match fixture
                .objref
                .to_raw_address()
                .as_usize()
                .checked_sub(offset)
            {
                Some(n) => unsafe { Address::from_usize(n) },
                None => break,
            };
            // It's just a smoke test.  It is hard to predict if the addr is still in any space,
            // but it must not crash.
            let _ = is_in_mmtk_spaces(ObjectReference::from_raw_address(addr));
        }
    });
}
