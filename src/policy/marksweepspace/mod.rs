//! Mark sweep space.
//! MMTk provides two implementations of mark sweep:
//! 1. mark sweep using a native freelist allocator implemented in MMTk. This is the default mark sweep implementation, and
//!    most people should use this.
//! 2. mark sweep using malloc as its freelist allocator. Use the feature `malloc_mark_sweep` to enable it. As we do not control
//!    the allocation of malloc, we have to work around a few issues to make it for mark sweep. Thus it has considerably worse performance.
//!    This is an experimental feature, and should only be used if you are actually interested in using malloc as the allocator.
//!    Otherwise this should not be used.

// TODO: we should extract the code about mark sweep, and make both implementation use the same mark sweep code.

// We will only use one of the two mark sweep implementations, depending on the enabled feature.
#![allow(dead_code)]

/// Malloc mark sweep. This uses `MallocSpace` and `MallocAllocator`.
pub(crate) mod malloc_ms;
/// Native mark sweep. This uses `MarkSweepSpace` and `FreeListAllocator`.
pub(crate) mod native_ms;
