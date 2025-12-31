use bytemuck::NoUninit;
use std::io::{Error, Result};

use crate::util::address::Address;

pub trait Memory {
    fn zero(start: Address, len: usize) {
        Self::set(start, 0, len);
    }
    fn set(start: Address, val: u8, len: usize) {
        unsafe {
            std::ptr::write_bytes::<u8>(start.to_mut_ptr(), val, len);
        }
    }
    fn dzmmap(start: Address, size: usize, strategy: MmapStrategy, annotation: &MmapAnnotation<'_>) -> Result<Address>;
    fn munmap(start: Address, size: usize) -> Result<()>;
    fn mprotect(start: Address, size: usize) -> Result<()>;
    fn munprotect(start: Address, size: usize) -> Result<()>;
}

/// Strategy for performing mmap
#[derive(Debug, Copy, Clone)]
pub struct MmapStrategy {
    /// Do we support huge pages?
    pub huge_page: HugePageSupport,
    /// The protection flags for mmap
    pub prot: MmapProtection,
    pub replace: bool,
    pub reserve: bool,
}

impl MmapStrategy {
    /// Create a new strategy
    pub fn new(transparent_hugepages: bool, prot: MmapProtection, replace: bool, reserve: bool) -> Self {
        Self {
            huge_page: if transparent_hugepages {
                HugePageSupport::TransparentHugePages
            } else {
                HugePageSupport::No
            },
            prot,
            replace,
            reserve,
        }
    }

    /// The strategy for MMTk's own internal memory
    pub const INTERNAL_MEMORY: Self = Self {
        huge_page: HugePageSupport::No,
        prot: MmapProtection::ReadWrite,
        replace: false,
        reserve: true,
    };

    /// The strategy for MMTk side metadata
    pub const SIDE_METADATA: Self = Self::INTERNAL_MEMORY;

    /// The strategy for MMTk's test memory
    #[cfg(test)]
    pub const TEST: Self = Self::INTERNAL_MEMORY;
}

/// The protection flags for Mmap
#[repr(i32)]
#[derive(Debug, Copy, Clone)]
pub enum MmapProtection {
    /// Allow read + write
    ReadWrite,
    /// Allow read + write + code execution
    ReadWriteExec,
    /// Do not allow any access
    NoAccess,
}

/// Support for huge pages
#[repr(u8)]
#[derive(Debug, Copy, Clone, NoUninit)]
pub enum HugePageSupport {
    /// No support for huge page
    No,
    /// Enable transparent huge pages for the pages that are mapped. This option is only for linux.
    TransparentHugePages,
}

/// Annotation for an mmap entry.
///
/// Invocations of `mmap_fixed` and other functions that may transitively call `mmap_fixed`
/// require an annotation that indicates the purpose of the memory mapping.
///
/// This is for debugging.  On Linux, mmtk-core will use `prctl` with `PR_SET_VMA` to set the
/// human-readable name for the given mmap region.  The annotation is ignored on other platforms.
///
/// Note that when using `Map32` (even when running on 64-bit architectures), the discontiguous
/// memory range is shared between different spaces. Spaces may use `mmap` to map new chunks, but
/// the same chunk may later be reused by other spaces. The annotation only applies when `mmap` is
/// called for a chunk for the first time, which reflects which space first attempted the mmap, not
/// which space is currently using the chunk.  Use `crate::policy::space::print_vm_map` to print a
/// more accurate mapping between address ranges and spaces.
///
/// On 32-bit architecture, side metadata are allocated in a chunked fasion.  One single `mmap`
/// region will contain many different metadata.  In that case, we simply annotate the whole region
/// with a `MmapAnnotation::SideMeta` where `meta` is `"all"`.
pub enum MmapAnnotation<'a> {
    /// The mmap is for a space.
    Space {
        /// The name of the space.
        name: &'a str,
    },
    /// The mmap is for a side metadata.
    SideMeta {
        /// The name of the space.
        space: &'a str,
        /// The name of the side metadata.
        meta: &'a str,
    },
    /// The mmap is for a test case.  Usually constructed using the [`mmap_anno_test!`] macro.
    Test {
        /// The source file.
        file: &'a str,
        /// The line number.
        line: u32,
    },
    /// For all other use cases.
    Misc {
        /// A human-readable descriptive name.
        name: &'a str,
    },
}

/// Construct an `MmapAnnotation::Test` with the current file name and line number.
#[macro_export]
macro_rules! mmap_anno_test_2 {
    () => {
        &$crate::util::memory::MmapAnnotation::Test {
            file: file!(),
            line: line!(),
        }
    };
}

// Export this to external crates
pub use mmap_anno_test_2;

impl std::fmt::Display for MmapAnnotation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MmapAnnotation::Space { name } => write!(f, "mmtk:space:{name}"),
            MmapAnnotation::SideMeta { space, meta } => write!(f, "mmtk:sidemeta:{space}:{meta}"),
            MmapAnnotation::Test { file, line } => write!(f, "mmtk:test:{file}:{line}"),
            MmapAnnotation::Misc { name } => write!(f, "mmtk:misc:{name}"),
        }
    }
}
