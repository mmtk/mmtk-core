use mmtk::vm::unboxed_size_constants::*;
use memory_manager_constants::*;

/* amount by which tracing causes headers to grow */
// XXX: workaround for not having const if-expressions
//pub const GC_TRACING_HEADER_WORDS: usize = if GENERATE_GC_TRACE { 3 } else { 0 };
pub const GC_TRACING_HEADER_WORDS: usize = if_then_else_zero_usize!(GENERATE_GC_TRACE, 3);
pub const GC_TRACING_HEADER_BYTES: usize = GC_TRACING_HEADER_WORDS << LOG_BYTES_IN_ADDRESS;

/**
 * How many bytes are used by all misc header fields?
 */
pub const NUM_BYTES_HEADER: usize = GC_TRACING_HEADER_BYTES; // + YYY_HEADER_BYTES;