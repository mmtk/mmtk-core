//! Memory policies that can be used for spaces.

/// This class defines and manages spaces.  Each policy is an instance
/// of a space.  A space is a region of virtual memory (contiguous or
/// discontigous) which is subject to the same memory management
/// regime.  Multiple spaces (instances of this class or its
/// descendants) may have the same policy (eg there could be numerous
/// instances of CopySpace, each with different roles). Spaces are
/// defined in terms of a unique region of virtual memory, so no two
/// space instances ever share any virtual memory.<p>
/// In addition to tracking virtual memory use and the mapping to
/// policy, spaces also manage memory consumption (<i>used</i> virtual
/// memory).
pub mod space;

/// Copy context defines the thread local copy allocator for copying policies.
pub mod copy_context;
/// Policy specific GC work
pub mod gc_work;
pub mod sft;
pub mod sft_map;

pub mod copyspace;
pub mod immix;
pub mod immortalspace;
pub mod largeobjectspace;
pub mod lockfreeimmortalspace;
pub mod markcompactspace;
pub mod marksweepspace;
