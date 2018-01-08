#[cfg(target_pointer_width = "32")]
pub const LOG_BYTES_IN_ADDRESS: usize = 2;
#[cfg(target_pointer_width = "64")]
pub const LOG_BYTES_IN_ADDRESS: usize = 3;
