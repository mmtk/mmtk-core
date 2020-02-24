use util::Address;
use libc::c_void;

pub fn zero(start: Address, len: usize) {
    unsafe {
        libc::memset(start.to_mut_ptr() as *mut libc::c_void, 0, len);
    }
}

pub fn dzmmap(start: Address, size: usize) -> i32 {
    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    let result: *mut c_void = unsafe { libc::mmap(start.to_mut_ptr::<c_void>(), size, prot, flags, -1, 0) };
    if Address::from_mut_ptr(result) == start {
        0
    } else {
        assert!(result as usize <= 127,
                "mmap with MAP_FIXED has unexpected behavior: demand zero mmap with MAP_FIXED on {:?} returned some other address {:?}",
                start, result
        );
        result as _
    }
}
