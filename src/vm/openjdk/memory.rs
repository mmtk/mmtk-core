use ::vm::Memory;
use libc;
use ::util::Address;

pub struct VMMemory;

impl Memory for VMMemory {
  fn dzmmap(start: Address, size: usize) -> i32 {
    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    let result = unsafe { Address::from_usize(libc::mmap(start.0 as _, size, prot, flags, -1, 0) as _) };
    if result == start {
      0
    } else {
      assert!(result.0 <= 127,
        "mmap with MAP_FIXED has unexpected behavior: demand zero mmap with MAP_FIXED on {:?} returned some other address {:?}",
        start, result
      );
      result.0 as _
    }
  }
}