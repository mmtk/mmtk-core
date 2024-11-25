use crate::util::alloc::AllocationError;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::{Collection, VMBinding};
use bytemuck::NoUninit;
use libc::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::io::{Error, Result};
use std::sync::atomic::AtomicBool;
use sysinfo::MemoryRefreshKind;
use sysinfo::{RefreshKind, System};

#[cfg(target_os = "linux")]
// MAP_FIXED_NOREPLACE returns EEXIST if already mapped
const MMAP_FLAGS: libc::c_int = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
#[cfg(target_os = "macos")]
// MAP_FIXED is used instead of MAP_FIXED_NOREPLACE (which is not available on macOS). We are at the risk of overwriting pre-existing mappings.
const MMAP_FLAGS: libc::c_int = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;

/// This static variable controls whether we annotate mmapped memory region using `prctl`. It can be
/// set via `Options::mmap_anno` or the `MMTK_MMAP_ANNO` environment variable.
///
/// FIXME: Since it is set via `Options`, it is in theory a decision per MMTk instance. However, we
/// currently don't have a good design for multiple MMTk instances, so we use static variable for
/// now.
pub(crate) static MMAP_ANNO: AtomicBool = AtomicBool::new(true);

/// Strategy for performing mmap
#[derive(Debug, Copy, Clone)]
pub struct MmapStrategy {
    /// Do we support huge pages?
    pub huge_page: HugePageSupport,
    /// The protection flags for mmap
    pub prot: MmapProtection,
}

impl MmapStrategy {
    /// Create a new strategy
    pub fn new(transparent_hugepages: bool, prot: MmapProtection) -> Self {
        Self {
            huge_page: if transparent_hugepages {
                HugePageSupport::TransparentHugePages
            } else {
                HugePageSupport::No
            },
            prot,
        }
    }

    /// The strategy for MMTk's own internal memory
    pub const INTERNAL_MEMORY: Self = Self {
        huge_page: HugePageSupport::No,
        prot: MmapProtection::ReadWrite,
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

impl MmapProtection {
    /// Turn the protection enum into the native flags
    pub fn into_native_flags(self) -> libc::c_int {
        match self {
            Self::ReadWrite => PROT_READ | PROT_WRITE,
            Self::ReadWriteExec => PROT_READ | PROT_WRITE | PROT_EXEC,
            Self::NoAccess => PROT_NONE,
        }
    }
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
/// Invocations of [`mmap_fixed`] and other functions that may transitively call [`mmap_fixed`]
/// require an annotation that indicates the purpose of the memory mapping.
///
/// This is for debugging.  On Linux, mmtk-core will use `prctl` with `PR_SET_VMA` to set the
/// human-readable name for the given mmap region.  The annotation is ignored on other platforms.
///
/// Note that when using `Map32` (even when running on 64-bit architectures), the discontiguous
/// memory range is shared between different spaces. Spaces may use `mmap` to map new chunks, but
/// the same chunk may later be reused by other spaces. The annotation only applies when `mmap` is
/// called for a chunk for the first time, which reflects which space first attempted the mmap, not
/// which space is currently using the chunk.  Use [`crate::policy::space::print_vm_map`] to print a
/// more accurate mapping between address ranges and spaces.
///
/// On 32-bit architecture, side metadata are allocated in a chunked fasion.  One single `mmap`
/// region will contain many different metadata.  In that case, we simply annotate the whole region
/// with a `MmapAnno::SideMeta` where `meta` is `"all"`.
pub enum MmapAnno<'a> {
    Space { name: &'a str },
    SideMeta { space: &'a str, meta: &'a str },
    Test { file: &'a str, line: u32 },
    Misc { name: &'a str },
}

#[macro_export]
macro_rules! mmap_anno_test {
    () => {
        &$crate::util::memory::MmapAnno::Test {
            file: file!(),
            line: line!(),
        }
    };
}

impl<'a> std::fmt::Display for MmapAnno<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MmapAnno::Space { name } => write!(f, "mmtk:space:{name}"),
            MmapAnno::SideMeta { space, meta } => write!(f, "mmtk:sidemeta:{space}:{meta}"),
            MmapAnno::Test { file, line } => write!(f, "mmtk:test:{file}:{line}"),
            MmapAnno::Misc { name } => write!(f, "mmtk:misc:{name}"),
        }
    }
}

/// Check the result from an mmap function in this module.
/// Return true if the mmap has failed due to an existing conflicting mapping.
pub(crate) fn result_is_mapped(result: Result<()>) -> bool {
    match result {
        Ok(_) => false,
        Err(err) => err.raw_os_error().unwrap() == libc::EEXIST,
    }
}

/// Set a range of memory to 0.
pub fn zero(start: Address, len: usize) {
    set(start, 0, len);
}

/// Set a range of memory to the given value. Similar to memset.
pub fn set(start: Address, val: u8, len: usize) {
    unsafe {
        std::ptr::write_bytes::<u8>(start.to_mut_ptr(), val, len);
    }
}

/// Demand-zero mmap:
/// This function mmaps the memory and guarantees to zero all mapped memory.
/// This function WILL overwrite existing memory mapping. The user of this function
/// needs to be aware of this, and use it cautiously.
///
/// # Safety
/// This function WILL overwrite existing memory mapping if there is any. So only use this function if you know
/// the memory has been reserved by mmtk (e.g. after the use of mmap_noreserve()). Otherwise using this function
/// may corrupt others' data.
#[allow(clippy::let_and_return)] // Zeroing is not neceesary for some OS/s
pub unsafe fn dzmmap(
    start: Address,
    size: usize,
    strategy: MmapStrategy,
    anno: &MmapAnno,
) -> Result<()> {
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    let ret = mmap_fixed(start, size, flags, strategy, anno);
    // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)
    #[cfg(not(target_os = "linux"))]
    if ret.is_ok() {
        zero(start, size)
    }
    ret
}
/// Demand-zero mmap (no replace):
/// This function mmaps the memory and guarantees to zero all mapped memory.
/// This function will not overwrite existing memory mapping, and it will result Err if there is an existing mapping.
#[allow(clippy::let_and_return)] // Zeroing is not neceesary for some OS/s
pub fn dzmmap_noreplace(
    start: Address,
    size: usize,
    strategy: MmapStrategy,
    anno: &MmapAnno,
) -> Result<()> {
    let flags = MMAP_FLAGS;
    let ret = mmap_fixed(start, size, flags, strategy, anno);
    // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)
    #[cfg(not(target_os = "linux"))]
    if ret.is_ok() {
        zero(start, size)
    }
    ret
}

/// mmap with no swap space reserve:
/// This function does not reserve swap space for this mapping, which means there is no guarantee that writes to the
/// mapping can always be successful. In case of out of physical memory, one may get a segfault for writing to the mapping.
/// We can use this to reserve the address range, and then later overwrites the mapping with dzmmap().
pub fn mmap_noreserve(
    start: Address,
    size: usize,
    mut strategy: MmapStrategy,
    anno: &MmapAnno,
) -> Result<()> {
    strategy.prot = MmapProtection::NoAccess;
    let flags = MMAP_FLAGS | libc::MAP_NORESERVE;
    mmap_fixed(start, size, flags, strategy, anno)
}

fn mmap_fixed(
    start: Address,
    size: usize,
    flags: libc::c_int,
    strategy: MmapStrategy,
    _anno: &MmapAnno,
) -> Result<()> {
    let ptr = start.to_mut_ptr();
    let prot = strategy.prot.into_native_flags();
    wrap_libc_call(
        &|| unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) },
        ptr,
    )?;

    #[cfg(target_os = "linux")]
    if MMAP_ANNO.load(std::sync::atomic::Ordering::SeqCst) {
        // `PR_SET_VMA` is new in Linux 5.17.  We compile against a version of the `libc` crate that
        // has the `PR_SET_VMA_ANON_NAME` constant.  When runnning on an older kernel, it will not
        // recognize this attribute and will return `EINVAL`.  However, `prctl` may return `EINVAL`
        // for other reasons, too.  That includes `start` being an invalid address, and the
        // formatted `anno_cstr` being longer than 80 bytes including the trailing `'\0'`.  But
        // since this prctl is used for debugging, we log the error instead of panicking.
        let anno_str = _anno.to_string();
        let anno_cstr = std::ffi::CString::new(anno_str).unwrap();
        let result = wrap_libc_call(
            &|| unsafe {
                libc::prctl(
                    libc::PR_SET_VMA,
                    libc::PR_SET_VMA_ANON_NAME,
                    start.to_ptr::<libc::c_void>(),
                    size,
                    anno_cstr.as_ptr(),
                )
            },
            0,
        );
        if let Err(e) = result {
            debug!("Error while calling prctl: {e}");
        }
    }

    match strategy.huge_page {
        HugePageSupport::No => Ok(()),
        HugePageSupport::TransparentHugePages => {
            #[cfg(target_os = "linux")]
            {
                wrap_libc_call(
                    &|| unsafe { libc::madvise(start.to_mut_ptr(), size, libc::MADV_HUGEPAGE) },
                    0,
                )
            }
            // Setting the transparent hugepage option to true will not pass
            // the validation on non-Linux OSes
            #[cfg(not(target_os = "linux"))]
            unreachable!()
        }
    }
}

/// Unmap the given memory (in page granularity). This wraps the unsafe libc munmap call.
pub fn munmap(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(&|| unsafe { libc::munmap(start.to_mut_ptr(), size) }, 0)
}

/// Properly handle errors from a mmap Result, including invoking the binding code in the case of
/// an OOM error.
pub fn handle_mmap_error<VM: VMBinding>(
    error: Error,
    tls: VMThread,
    addr: Address,
    bytes: usize,
) -> ! {
    use std::io::ErrorKind;

    eprintln!("Failed to mmap {}, size {}", addr, bytes);
    eprintln!("{}", get_process_memory_maps());

    match error.kind() {
        // From Rust nightly 2021-05-12, we started to see Rust added this ErrorKind.
        ErrorKind::OutOfMemory => {
            // Signal `MmapOutOfMemory`. Expect the VM to abort immediately.
            trace!("Signal MmapOutOfMemory!");
            VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
            unreachable!()
        }
        // Before Rust had ErrorKind::OutOfMemory, this is how we capture OOM from OS calls.
        // TODO: We may be able to remove this now.
        ErrorKind::Other => {
            // further check the error
            if let Some(os_errno) = error.raw_os_error() {
                // If it is OOM, we invoke out_of_memory() through the VM interface.
                if os_errno == libc::ENOMEM {
                    // Signal `MmapOutOfMemory`. Expect the VM to abort immediately.
                    trace!("Signal MmapOutOfMemory!");
                    VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
                    unreachable!()
                }
            }
        }
        ErrorKind::AlreadyExists => {
            panic!("Failed to mmap, the address is already mapped. Should MMTk quarantine the address range first?");
        }
        _ => {}
    }
    panic!("Unexpected mmap failure: {:?}", error)
}

/// Checks if the memory has already been mapped. If not, we panic.
///
/// Note that the checking has a side effect that it will map the memory if it was unmapped. So we panic if it was unmapped.
/// Be very careful about using this function.
///
/// This function is currently left empty for non-linux, and should be implemented in the future.
/// As the function is only used for assertions, MMTk will still run even if we never panic.
pub(crate) fn panic_if_unmapped(_start: Address, _size: usize, _anno: &MmapAnno) {
    #[cfg(target_os = "linux")]
    {
        let flags = MMAP_FLAGS;
        match mmap_fixed(
            _start,
            _size,
            flags,
            MmapStrategy {
                huge_page: HugePageSupport::No,
                prot: MmapProtection::ReadWrite,
            },
            _anno,
        ) {
            Ok(_) => panic!("{} of size {} is not mapped", _start, _size),
            Err(e) => {
                assert!(
                    e.kind() == std::io::ErrorKind::AlreadyExists,
                    "Failed to check mapped: {:?}",
                    e
                );
            }
        }
    }
}

/// Unprotect the given memory (in page granularity) to allow access (PROT_READ/WRITE/EXEC).
pub fn munprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
    let prot = prot.into_native_flags();
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, prot) },
        0,
    )
}

/// Protect the given memory (in page granularity) to forbid any access (PROT_NONE).
pub fn mprotect(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_NONE) },
        0,
    )
}

fn wrap_libc_call<T: PartialEq>(f: &dyn Fn() -> T, expect: T) -> Result<()> {
    let ret = f();
    if ret == expect {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Get the memory maps for the process. The returned string is a multi-line string.
/// This is only meant to be used for debugging. For example, log process memory maps after detecting a clash.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn get_process_memory_maps() -> String {
    // print map
    use std::fs::File;
    use std::io::Read;
    let mut data = String::new();
    let mut f = File::open("/proc/self/maps").unwrap();
    f.read_to_string(&mut data).unwrap();
    data
}

/// Get the memory maps for the process. The returned string is a multi-line string.
/// This is only meant to be used for debugging. For example, log process memory maps after detecting a clash.
#[cfg(target_os = "macos")]
pub fn get_process_memory_maps() -> String {
    // Get the current process ID (replace this with a specific PID if needed)
    let pid = std::process::id();

    // Execute the vmmap command
    let output = std::process::Command::new("vmmap")
        .arg(pid.to_string()) // Pass the PID as an argument
        .output() // Capture the output
        .expect("Failed to execute vmmap command");

    // Check if the command was successful
    if output.status.success() {
        // Convert the command output to a string
        let output_str =
            std::str::from_utf8(&output.stdout).expect("Failed to convert output to string");
        output_str.to_string()
    } else {
        // Handle the error case
        let error_message =
            std::str::from_utf8(&output.stderr).expect("Failed to convert error message to string");
        panic!("Failed to get process memory map: {}", error_message)
    }
}

/// Get the memory maps for the process. The returned string is a multi-line string.
/// This is only meant to be used for debugging. For example, log process memory maps after detecting a clash.
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
pub fn get_process_memory_maps() -> String {
    "(process map unavailable)".to_string()
}

/// Returns the total physical memory for the system in bytes.
pub(crate) fn get_system_total_memory() -> u64 {
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
        RefreshKind::new().with_memory(MemoryRefreshKind::new().with_ram()),
    );
    sys.total_memory()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::constants::BYTES_IN_PAGE;
    use crate::util::test_util::MEMORY_TEST_REGION;
    use crate::util::test_util::{serial_test, with_cleanup};

    // In the tests, we will mmap this address. This address should not be in our heap (in case we mess up with other tests)
    const START: Address = MEMORY_TEST_REGION.start;

    #[test]
    fn test_mmap() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = unsafe {
                        dzmmap(START, BYTES_IN_PAGE, MmapStrategy::TEST, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                    // We can overwrite with dzmmap
                    let res = unsafe {
                        dzmmap(START, BYTES_IN_PAGE, MmapStrategy::TEST, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            );
        });
    }

    #[test]
    fn test_munmap() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = dzmmap_noreplace(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!(),
                    );
                    assert!(res.is_ok());
                    let res = munmap(START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_mmap_noreplace() {
        serial_test(|| {
            with_cleanup(
                || {
                    // Make sure we mmapped the memory
                    let res = unsafe {
                        dzmmap(START, BYTES_IN_PAGE, MmapStrategy::TEST, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                    // Use dzmmap_noreplace will fail
                    let res = dzmmap_noreplace(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!(),
                    );
                    assert!(res.is_err());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        });
    }

    #[test]
    fn test_mmap_noreserve() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res =
                        mmap_noreserve(START, BYTES_IN_PAGE, MmapStrategy::TEST, mmap_anno_test!());
                    assert!(res.is_ok());
                    // Try reserve it
                    let res = unsafe {
                        dzmmap(START, BYTES_IN_PAGE, MmapStrategy::TEST, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[should_panic]
    fn test_check_is_mmapped_for_unmapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We expect this call to panic
                    panic_if_unmapped(START, BYTES_IN_PAGE, mmap_anno_test!());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[test]
    fn test_check_is_mmapped_for_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    assert!(dzmmap_noreplace(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!()
                    )
                    .is_ok());
                    panic_if_unmapped(START, BYTES_IN_PAGE, mmap_anno_test!());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[should_panic]
    fn test_check_is_mmapped_for_unmapped_next_to_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    // map 1 page from START
                    assert!(dzmmap_noreplace(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!(),
                    )
                    .is_ok());

                    // check if the next page is mapped - which should panic
                    panic_if_unmapped(START + BYTES_IN_PAGE, BYTES_IN_PAGE, mmap_anno_test!());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE * 2).is_ok());
                },
            )
        })
    }

    #[test]
    #[should_panic]
    // This is a bug we need to fix. We need to figure out a way to properly check if a piece of memory is mapped or not.
    // Alternatively, we should remove the code that calls the function.
    #[ignore]
    fn test_check_is_mmapped_for_partial_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    // map 1 page from START
                    assert!(dzmmap_noreplace(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!()
                    )
                    .is_ok());

                    // check if the 2 pages from START are mapped. The second page is unmapped, so it should panic.
                    panic_if_unmapped(START, BYTES_IN_PAGE * 2, mmap_anno_test!());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE * 2).is_ok());
                },
            )
        })
    }

    #[test]
    fn test_get_system_total_memory() {
        let total = get_system_total_memory();
        println!("Total memory: {:?}", total);
    }
}
