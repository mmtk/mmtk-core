// GITHUB-CI: FEATURES=malloc_counted_size

use crate::api::*;
use crate::test_fixtures::{MMTKSingleton, SerialFixture};

lazy_static! {
    static ref MMTK_SINGLETON: SerialFixture<MMTKSingleton> = SerialFixture::new();
}

#[test]
pub fn malloc_free() {
    MMTK_SINGLETON.with_fixture(|_| {
        let bytes_before = mmtk_get_malloc_bytes();

        let res = mmtk_counted_malloc(8);
        assert!(!res.is_zero());
        let bytes_after_alloc = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before + 8, bytes_after_alloc);

        mmtk_free_with_size(res, 8);
        let bytes_after_free = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before, bytes_after_free);
    });
}

#[test]
pub fn calloc_free() {
    MMTK_SINGLETON.with_fixture(|_| {
        let bytes_before = mmtk_get_malloc_bytes();

        let res = mmtk_counted_calloc(1, 8);
        assert!(!res.is_zero());
        let bytes_after_alloc = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before + 8, bytes_after_alloc);

        mmtk_free_with_size(res, 8);
        let bytes_after_free = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before, bytes_after_free);
    });
}

#[test]
pub fn realloc_grow() {
    MMTK_SINGLETON.with_fixture(|_| {
        let bytes_before = mmtk_get_malloc_bytes();

        let res1 = mmtk_counted_malloc(8);
        assert!(!res1.is_zero());
        let bytes_after_alloc = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before + 8, bytes_after_alloc);

        // grow to 16 bytes
        let res2 = mmtk_realloc_with_old_size(res1, 16, 8);
        assert!(!res2.is_zero());
        let bytes_after_realloc = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before + 16, bytes_after_realloc);

        mmtk_free_with_size(res2, 16);
        let bytes_after_free = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before, bytes_after_free);
    });
}

#[test]
pub fn realloc_shrink() {
    MMTK_SINGLETON.with_fixture(|_| {
        let bytes_before = mmtk_get_malloc_bytes();

        let res1 = mmtk_counted_malloc(16);
        assert!(!res1.is_zero());
        let bytes_after_alloc = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before + 16, bytes_after_alloc);

        // shrink to 8 bytes
        let res2 = mmtk_realloc_with_old_size(res1, 8, 16);
        assert!(!res2.is_zero());
        let bytes_after_realloc = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before + 8, bytes_after_realloc);

        mmtk_free_with_size(res2, 8);
        let bytes_after_free = mmtk_get_malloc_bytes();
        assert_eq!(bytes_before, bytes_after_free);
    });
}
