use self::entrypoint::*;
pub use self::scheduling::BOOT_THREAD;

mod entrypoint;

#[macro_use]
mod jtoc_call;

pub mod scanning;
pub mod scheduling;
pub mod object_model;
pub mod unboxed_size_constants;
pub mod java_header;
pub mod java_size_constants;
pub mod java_header_constants;
pub mod memory_manager_constants;
pub mod misc_header_constants;
pub mod scan_statics;
pub mod scan_boot_image;
pub mod active_plan;

use ::util::address::Address;

pub static mut JTOC_BASE: Address = Address(0);

pub struct JikesRVM {}

impl JikesRVM {
    #[inline(always)]
    pub fn test(input: usize) -> usize {
        unsafe {
            jtoc_call!(TEST_METHOD_OFFSET, BOOT_THREAD, input)
        }
    }

    #[inline(always)]
    pub fn test1() -> usize {
        unsafe {
            jtoc_call!(TEST1_METHOD_OFFSET, BOOT_THREAD)
        }
    }

    #[inline(always)]
    pub fn test2(input1: usize, input2: usize) -> usize {
        unsafe {
            jtoc_call!(TEST2_METHOD_OFFSET, BOOT_THREAD, input1, input2)
        }
    }

    #[inline(always)]
    pub fn test3(input1: usize, input2: usize, input3: usize, input4: usize) -> usize {
        unsafe {
            jtoc_call!(TEST3_METHOD_OFFSET, BOOT_THREAD, input1, input2, input3, input4)
        }
    }
}