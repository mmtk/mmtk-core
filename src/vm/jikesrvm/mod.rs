use self::jtoc::*;
pub use self::scheduling::BOOT_THREAD;

mod jtoc;

#[macro_use]
mod jtoc_call;

pub mod scanning;
pub mod scheduling;
pub mod object_model;
pub mod unboxed_size_constants;
pub mod scan_statics;

use ::util::address::Address;

pub static mut JTOC_BASE: Address = Address(0);

pub struct JikesRVM {}

impl JikesRVM {
    #[inline(always)]
    pub fn test(input: usize) -> usize {
        jtoc_call!(TEST_METHOD_JTOC_OFFSET, BOOT_THREAD, input)
    }

    #[inline(always)]
    pub fn test1() -> usize {
        jtoc_call!(TEST1_METHOD_JTOC_OFFSET, BOOT_THREAD)
    }

    #[inline(always)]
    pub fn test2(input1: usize, input2: usize) -> usize {
        jtoc_call!(TEST2_METHOD_JTOC_OFFSET, BOOT_THREAD, input1, input2)
    }

    #[inline(always)]
    pub fn test3(input1: usize, input2: usize, input3: usize, input4: usize) -> usize {
        jtoc_call!(TEST3_METHOD_JTOC_OFFSET, BOOT_THREAD, input1, input2, input3, input4)
    }
}