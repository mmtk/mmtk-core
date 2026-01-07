use bytemuck::NoUninit;
use std::io::Result;

use crate::util::os::*;
use crate::vm::*;
use crate::{
    util::{address::Address, VMThread},
    vm::VMBinding,
};

/// Abstraction for OS memory operations.
pub trait Memory {
    /// Set a memory region to zero.
    fn zero(start: Address, len: usize) {
        Self::set(start, 0, len);
    }

    /// Set a memory region to a specific value.
    fn set(start: Address, val: u8, len: usize) {
        unsafe {
            std::ptr::write_bytes::<u8>(start.to_mut_ptr(), val, len);
        }
    }

    /// Perform a demand-zero mmap.
    ///
    /// Falback: `annotation` is only used for debugging. For platforms that do not support mmap annotations, this parameter can be ignored.
    fn dzmmap(
        start: Address,
        size: usize,
        strategy: MmapStrategy,
        annotation: &MmapAnnotation<'_>,
    ) -> Result<Address>;

    /// Handle mmap errors, possibly by signaling the VM about an out-of-memory condition.
    fn handle_mmap_error<VM: VMBinding>(
        error: std::io::Error,
        tls: VMThread,
        addr: Address,
        bytes: usize,
    ) {
        use crate::util::alloc::AllocationError;
        use std::io::ErrorKind;

        eprintln!("Failed to mmap {}, size {}", addr, bytes);
        eprintln!("{}", OSProcess::get_process_memory_maps().unwrap());

        let call_binding_oom = || {
            // Signal `MmapOutOfMemory`. Expect the VM to abort immediately.
            trace!("Signal MmapOutOfMemory!");
            VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
            unreachable!()
        };

        match error.kind() {
            // From Rust nightly 2021-05-12, we started to see Rust added this ErrorKind.
            ErrorKind::OutOfMemory => {
                call_binding_oom();
            }
            // Before Rust had ErrorKind::OutOfMemory, this is how we capture OOM from OS calls.
            // TODO: We may be able to remove this now.
            ErrorKind::Other => {
                // further check the error
                if let Some(os_errno) = error.raw_os_error() {
                    if OSMemory::is_mmap_oom(os_errno) {
                        call_binding_oom();
                    }
                }
            }
            ErrorKind::AlreadyExists => {
                panic!("Failed to mmap, the address is already mapped. Should MMTk quarantine the address range first?");
            }
            _ => {
                if let Some(os_errno) = error.raw_os_error() {
                    if OSMemory::is_mmap_oom(os_errno) {
                        call_binding_oom();
                    }
                }
            }
        }
        panic!("Unexpected mmap failure: {:?}", error)
    }

    /// Check whether the given OS error number indicates an out-of-memory condition.
    fn is_mmap_oom(os_errno: i32) -> bool;

    /// Unmap a memory region.
    fn munmap(start: Address, size: usize) -> Result<()>;

    /// Change the protection of a memory region to no access to forbit any access to the memory region.
    fn mprotect(start: Address, size: usize) -> Result<()>;

    /// Change the protection of a memory region to the specified protection.
    fn munprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()>;

    /// Checks if the memory has already been mapped. If not, we panic.
    ///
    /// Note that the checking may have a side effect that it will map the memory if it was unmapped. So we panic if it was unmapped.
    /// Be very careful about using this function.
    ///
    /// Fallback: As the function is only used for assertions, it can be a no-op, and MMTk will still run and never panics in this function.
    fn panic_if_unmapped(start: Address, size: usize);

    /// Get the total memory of the system in bytes.
    fn get_system_total_memory() -> Result<u64> {
        use sysinfo::MemoryRefreshKind;
        use sysinfo::{RefreshKind, System};

        // TODO: Note that if we want to get system info somewhere else in the future, we should
        // refactor this instance into some global struct. sysinfo recommends sharing one instance of
        // `System` instead of making multiple instances.
        // See https://docs.rs/sysinfo/0.29.0/sysinfo/index.html#usage for more info
        //
        // If we refactor the `System` instance to use it for other purposes, please make sure start-up
        // time is not affected.  It takes a long time to load all components in sysinfo (e.g. by using
        // `System::new_all()`).  Some applications, especially short-running scripts, are sensitive to
        // start-up time.  During start-up, MMTk core only needs the total memory to initialize the
        // `Options`.  If we only load memory-related components on start-up, it should only take <1ms
        // to initialize the `System` instance.
        let sys = System::new_with_specifics(
            RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        Ok(sys.total_memory())
    }
}

/// Strategy for performing mmap
#[derive(Debug, Copy, Clone)]
pub struct MmapStrategy {
    /// Whether we should use huge page for this mmapping.
    /// Fallback: for platforms that do not support huge pages, this option can be ignored.
    pub huge_page: HugePageSupport,
    /// The protection flags for mmap.
    pub prot: MmapProtection,
    /// Whether this mmap allows replacing existing mappings.
    /// Fallback: for platforms that cannot replace existing mappings, or always replace existing mappings, this option can be ignored.
    pub replace: bool,
    /// Whether this mmap allows reserve/commit physical memory.
    /// This has to be implemented properly for a platform. Otherwise, we will see huge unrealistic memory consumption.
    pub reserve: bool,
}

impl std::default::Default for MmapStrategy {
    fn default() -> Self {
        Self {
            huge_page: HugePageSupport::No,
            prot: MmapProtection::ReadWrite,
            replace: false,
            reserve: true,
        }
    }
}

impl MmapStrategy {
    /// Create a new strategy
    pub fn new(
        huge_page: HugePageSupport,
        prot: MmapProtection,
        replace: bool,
        reserve: bool,
    ) -> Self {
        Self {
            huge_page,
            prot,
            replace,
            reserve,
        }
    }

    // Builder methods

    /// Set huge page option.
    pub fn huge_page(self, huge_page: HugePageSupport) -> Self {
        Self { huge_page, ..self }
    }

    /// Set huge page option by a boolean flag.
    pub fn transparent_hugepages(self, enable: bool) -> Self {
        let huge_page = if enable {
            HugePageSupport::TransparentHugePages
        } else {
            HugePageSupport::No
        };
        Self { huge_page, ..self }
    }

    /// Set protection option.
    pub fn prot(self, prot: MmapProtection) -> Self {
        Self { prot, ..self }
    }

    /// Set the replace flag.
    pub fn replace(self, replace: bool) -> Self {
        Self { replace, ..self }
    }

    /// Set the reserve flag.
    pub fn reserve(self, reserve: bool) -> Self {
        Self { reserve, ..self }
    }

    #[cfg(test)] // In test mode, we use test settings which allows replacing existing mappings.
    /// The strategy for MMTk's own internal memory (test)
    pub const INTERNAL_MEMORY: Self = Self::TEST;
    #[cfg(not(test))]
    /// The strategy for MMTk's own internal memory
    pub const INTERNAL_MEMORY: Self = Self {
        huge_page: HugePageSupport::No,
        prot: MmapProtection::ReadWrite,
        replace: false,
        reserve: true,
    };

    #[cfg(test)]
    /// The strategy for MMTk side metadata (test)
    pub const SIDE_METADATA: Self = Self::TEST;
    #[cfg(not(test))]
    /// The strategy for MMTk side metadata
    pub const SIDE_METADATA: Self = Self::INTERNAL_MEMORY;

    /// The strategy for MMTk's test memory
    #[cfg(test)]
    pub const TEST: Self = Self {
        huge_page: HugePageSupport::No,
        prot: MmapProtection::ReadWrite,
        replace: true,
        reserve: true,
    };
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
macro_rules! mmap_anno_test {
    () => {
        &$crate::util::os::MmapAnnotation::Test {
            file: file!(),
            line: line!(),
        }
    };
}

// Export this to external crates
pub use mmap_anno_test;

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
