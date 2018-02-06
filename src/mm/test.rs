
#[no_mangle]
#[cfg(feature = "jikesrvm")]
pub extern fn test_stack_alignment() {
    trace!{"Entering stack alignment test with no args passed"}
    unsafe {
        asm!("movaps %xmm1, (%esp)" : : : "sp", "%xmm1", "memory");
    }
    trace!{"Exiting stack alignment test"}
}

#[no_mangle]
#[cfg(feature = "jikesrvm")]
pub extern fn test_stack_alignment1(a: usize, b: usize, c: usize, d: usize, e: usize) -> usize {
    trace!{"Entering stack alignment test"}
    trace!{"a:{} , b:{}, c:{}, d:{}, e:{}",
           a, b, c, d, e}
    unsafe {
        asm!("movaps %xmm1, (%esp)" : : : "sp", "%xmm1", "memory");
    }
    let result = a + b * 2 + c * 3  + d * 4 + e * 5;
    trace!{"Exiting stack alignment test"}
    result
}