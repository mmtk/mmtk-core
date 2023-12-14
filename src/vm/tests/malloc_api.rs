use crate::memory_manager;

#[test]
pub fn malloc_free() {
    let res = memory_manager::malloc(8);
    assert!(!res.is_zero());
    memory_manager::free(res);
}

#[test]
pub fn calloc_free() {
    let res = memory_manager::calloc(1, 8);
    assert!(!res.is_zero());
    memory_manager::free(res);
}

#[test]
pub fn realloc_free() {
    let res1 = memory_manager::malloc(8);
    assert!(!res1.is_zero());
    let res2 = memory_manager::realloc(res1, 16);
    assert!(!res2.is_zero());
    memory_manager::free(res2);
}
