// GITHUB-CI: FEATURES=malloc_counted_size

use super::mock_test_prelude::*;

lazy_static! {
    static ref MMTK: Fixture<MMTKFixture> = Fixture::new();
}

#[test]
pub fn malloc_free() {
    with_mockvm(
        default_setup,
        || {
            MMTK.with_fixture(|fixture| {
                let bytes_before = memory_manager::get_malloc_bytes(fixture.get_mmtk());

                let res = memory_manager::counted_malloc(fixture.get_mmtk(), 8);
                assert!(!res.is_zero());
                let bytes_after_alloc = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before + 8, bytes_after_alloc);

                memory_manager::free_with_size(fixture.get_mmtk(), res, 8);
                let bytes_after_free = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before, bytes_after_free);
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn calloc_free() {
    with_mockvm(
        default_setup,
        || {
            MMTK.with_fixture(|fixture| {
                let bytes_before = memory_manager::get_malloc_bytes(fixture.get_mmtk());

                let res = memory_manager::counted_calloc(fixture.get_mmtk(), 1, 8);
                assert!(!res.is_zero());
                let bytes_after_alloc = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before + 8, bytes_after_alloc);

                memory_manager::free_with_size(fixture.get_mmtk(), res, 8);
                let bytes_after_free = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before, bytes_after_free);
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn realloc_grow() {
    with_mockvm(
        default_setup,
        || {
            MMTK.with_fixture(|fixture| {
                let bytes_before = memory_manager::get_malloc_bytes(fixture.get_mmtk());

                let res1 = memory_manager::counted_malloc(&fixture.get_mmtk(), 8);
                assert!(!res1.is_zero());
                let bytes_after_alloc = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before + 8, bytes_after_alloc);

                // grow to 16 bytes
                let res2 = memory_manager::realloc_with_old_size(fixture.get_mmtk(), res1, 16, 8);
                assert!(!res2.is_zero());
                let bytes_after_realloc = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before + 16, bytes_after_realloc);

                memory_manager::free_with_size(&fixture.get_mmtk(), res2, 16);
                let bytes_after_free = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before, bytes_after_free);
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn realloc_shrink() {
    with_mockvm(
        default_setup,
        || {
            MMTK.with_fixture(|fixture| {
                let bytes_before = memory_manager::get_malloc_bytes(fixture.get_mmtk());

                let res1 = memory_manager::counted_malloc(fixture.get_mmtk(), 16);
                assert!(!res1.is_zero());
                let bytes_after_alloc = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before + 16, bytes_after_alloc);

                // shrink to 8 bytes
                let res2 = memory_manager::realloc_with_old_size(fixture.get_mmtk(), res1, 8, 16);
                assert!(!res2.is_zero());
                let bytes_after_realloc = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before + 8, bytes_after_realloc);

                memory_manager::free_with_size(fixture.get_mmtk(), res2, 8);
                let bytes_after_free = memory_manager::get_malloc_bytes(fixture.get_mmtk());
                assert_eq!(bytes_before, bytes_after_free);
            });
        },
        no_cleanup,
    )
}
