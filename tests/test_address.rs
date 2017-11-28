extern crate mmtk;

use mmtk::address::Address;

#[test]
fn test_align_up() {
    let addr = Address(0);
    let aligned = addr.align_up(8);

    assert_eq!(addr, aligned);
}

#[test]
fn test_is_aligned() {
    let addr = Address(0);
    assert!(addr.is_aligned_to(8));

    let addr = Address(8);
    assert!(addr.is_aligned_to(8));
}