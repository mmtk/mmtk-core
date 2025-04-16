use mmtk::util::Address;

#[test]
fn test_align_up() {
    let addr = unsafe { Address::zero() };
    let aligned = addr.align_up(8);

    assert_eq!(addr, aligned);
}

#[test]
fn test_is_aligned() {
    let addr = unsafe { Address::zero() };
    assert!(addr.is_aligned_to(8));

    let addr = unsafe { Address::from_usize(8) };
    assert!(addr.is_aligned_to(8));
}
