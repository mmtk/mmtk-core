/// Number of bits in space indices.
pub const SPACE_INDEX_BITS: usize = 4;

/// Limit of the space indices.  Any valid space index must be strictly less than this value.
pub const SPACE_INDEX_LIMIT: usize = 1 << SPACE_INDEX_BITS;

/// The bit mask for space indices.
pub const SPACE_INDEX_MASK: usize = SPACE_INDEX_LIMIT - 1;

/// Maximum number of spaces a Plan can support.
///
/// Index 0 is not used.  Valid indices are `1 ..= SPACE_INDEX_LIMIT - 1`.
pub const MAX_SPACES: usize = SPACE_INDEX_LIMIT - 1;
