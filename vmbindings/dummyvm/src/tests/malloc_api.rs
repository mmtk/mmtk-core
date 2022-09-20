use crate::api::*;

#[test]
pub fn malloc_free() {
    let res = mmtk_malloc(8);
    assert!(!res.is_zero());
    mmtk_free(res);
}

#[test]
pub fn calloc_free() {
    let res = mmtk_calloc(1, 8);
    assert!(!res.is_zero());
    mmtk_free(res);
}

#[test]
pub fn realloc_free() {
    let res1 = mmtk_malloc(8);
    assert!(!res1.is_zero());
    let res2 = mmtk_realloc(res1, 16);
    assert!(!res2.is_zero());
    mmtk_free(res2);
}
