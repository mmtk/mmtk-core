// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;

use crate::util::*;

lazy_static! {
    static ref SINGLE_OBJECT: Fixture<SingleObject> = Fixture::new();
}

#[test]
pub fn null() {
    with_mockvm(
        default_setup,
        || {
            SINGLE_OBJECT.with_fixture(|_| {
                assert!(
                    !memory_manager::is_in_mmtk_spaces::<MockVM>(ObjectReference::NULL),
                    "NULL pointer should not be in any MMTk spaces."
                );
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
            SINGLE_OBJECT.with_fixture(|_fixture| {
                assert!(
                    !memory_manager::is_in_mmtk_spaces::<MockVM>(
                        ObjectReference::from_raw_address(Address::MAX)
                    ),
                    "Address::MAX should not be in any MMTk spaces."
                );
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
                assert!(
                    memory_manager::is_in_mmtk_spaces::<MockVM>(fixture.objref),
                    "The address of the allocated object should be in the space"
                );
            });
        },
        no_cleanup,
    )
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
                    // It's just a smoke test.  It is hard to predict if the addr is still in any space,
                    // but it must not crash.
                    let _ = memory_manager::is_in_mmtk_spaces::<MockVM>(
                        ObjectReference::from_raw_address(addr),
                    );
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
                    let _ = memory_manager::is_in_mmtk_spaces::<MockVM>(
                        ObjectReference::from_raw_address(addr),
                    );
                }
            });
        },
        no_cleanup,
    )
}
