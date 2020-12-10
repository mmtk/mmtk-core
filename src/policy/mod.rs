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

#[cfg(feature = "immortalspace")]
pub mod immortalspace;

#[cfg(feature = "copyspace")]
pub mod copyspace;

#[cfg(feature = "largeobjectspace")]
pub mod largeobjectspace;

#[cfg(feature = "lockfreeimmortalspace")]
pub mod lockfreeimmortalspace;

pub use space::NODES;
