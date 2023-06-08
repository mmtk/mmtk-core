//! This module updates of VO bits for ImmixSpace during GC.
//! The handling is very sensitive to `ImmixVOBitUpdateStrategy`, and may be a bit verbose.
//! We abstract VO-bit-related code out of the main parts of the Immix algorithm to make it more
//! readable.


