use std::ptr::null_mut;
use ::mm::memory_manager::OpenJDK_Upcalls;

pub mod scanning;
pub mod collection;
pub mod object_model;
pub mod active_plan;
pub mod reference_glue;
pub mod memory;

pub static mut UPCALLS: *const OpenJDK_Upcalls = null_mut();