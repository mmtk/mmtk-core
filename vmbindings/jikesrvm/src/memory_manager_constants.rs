use mmtk::vm::unboxed_size_constants::*;
use mmtk::SelectedConstraints;

/** {@code true} if the selected plan needs support for linearly scanning the heap */
pub const NEEDS_LINEAR_SCAN: bool = SelectedConstraints::NEEDS_LINEAR_SCAN;
/** Number of bits in the GC header required by the selected plan */
pub const GC_HEADER_BITS: usize = SelectedConstraints::GC_HEADER_BITS;
/** Number of additional bytes required in the header by the selected plan */
pub const GC_HEADER_BYTES: usize = SelectedConstraints::GC_HEADER_WORDS << LOG_BYTES_IN_WORD;
/** {@code true} if the selected plan requires concurrent worker threads */
pub const NEEDS_CONCURRENT_WORKERS: bool = SelectedConstraints::NEEDS_CONCURRENT_WORKERS;
/** {@code true} if the selected plan needs support for generating a GC trace */
pub const GENERATE_GC_TRACE: bool = SelectedConstraints::GENERATE_GC_TRACE;
/** {@code true} if the selected plan may move objects */
pub const MOVES_OBJECTS: bool = SelectedConstraints::MOVES_OBJECTS;
/** {@code true} if the selected plan moves TIB objects */
pub const MOVES_TIBS: bool = false;
/** {@code true} if the selected plan moves code */
pub const MOVES_CODE: bool = false;