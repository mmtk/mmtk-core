/**
 * log_2 of the maximum number of spaces a Plan can support.
 */
#[cfg(target_pointer_width = "32")]
pub const LOG_MAX_SPACES: usize = 4;
#[cfg(target_pointer_width = "64")]
pub const LOG_MAX_SPACES: usize = 47 - 41;

/**
 * Maximum number of spaces a Plan can support.
 */
pub const MAX_SPACES: usize = 1 << LOG_MAX_SPACES;
