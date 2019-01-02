use ::vm::Memory;
use libc;
use ::util::Address;


const PROT_NONE: i32 = 0;
const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;

const MAP_PRIVATE: i32 = 2;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAP_FIXED: i32 = 16;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const MAP_FIXED: i32 = 256;

#[cfg(target_os = "linux")]
const MAP_ANONYMOUS: i32 = 32;
#[cfg(target_os = "macos")]
const MAP_ANONYMOUS: i32 = 0x1000;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const MAP_ANONYMOUS: i32 = 16;


pub struct VMMemory;

impl Memory for VMMemory {
  fn dzmmap(start: Address, size: usize) -> i32 {
    let prot = PROT_READ | PROT_WRITE | PROT_EXEC;
    let flags = MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED;
    let result = Address(unsafe { libc::mmap(start.0 as _, size, prot, flags, -1, 0) } as _);
    if result == start {
      0
    } else {
      assert!(result <= Address(127),
        "mmap with MAP_FIXED has unexpected behavior: demand zero mmap with MAP_FIXED on {:?} returned some other address {:?}",
        start, result
      );
      result.0 as _
    }
  }
}