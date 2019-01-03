use ::util::Address;
use libc;

pub trait Memory {
  fn dzmmap(start: Address, size: usize) -> i32;
  fn zero(start: Address, len: usize) {
    unsafe {
      libc::memset(start.to_ptr_mut() as *mut libc::c_void, 0, len);
    }
  }
}
