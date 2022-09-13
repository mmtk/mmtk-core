/**
 * log_2 of the maximum number of spaces a Plan can support.
 */
pub const LOG_MAX_SPACES: usize = 4;

/**
 * Maximum number of spaces a Plan can support.
 */
pub const MAX_SPACES: usize = 1 << LOG_MAX_SPACES;

/**
 * In a 64-bit addressing model, each space is the same size, given
 * by this constant.  At the moment, we require that the number of
 * pages in a space fit into a 32-bit signed int, so the maximum
 * size of this constant is 41 (assuming 4k pages).
 */
pub const LOG_SPACE_SIZE_64: usize = 41;
type a = UnimplementedMemorySlice;
