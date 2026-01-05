use crate::util::alloc::AllocationError;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::{Collection, VMBinding};
use bytemuck::NoUninit;
#[cfg(not(target_os = "windows"))]
use libc::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::io::{Error, Result};
use sysinfo::MemoryRefreshKind;
use sysinfo::{RefreshKind, System};

#[cfg(target_os = "linux")]
// MAP_FIXED_NOREPLACE returns EEXIST if already mapped
const MMAP_FLAGS: libc::c_int = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
#[cfg(target_os = "macos")]
// MAP_FIXED is used instead of MAP_FIXED_NOREPLACE (which is not available on macOS). We are at the risk of overwriting pre-existing mappings.
const MMAP_FLAGS: libc::c_int = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
#[cfg(target_os = "windows")]
const MMAP_FLAGS: libc::c_int = 0; // Not used on Windows
#[cfg(target_os = "windows")]
const MAP_NORESERVE: libc::c_int = 0x4000; // Custom flag for Windows emulation

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
    /// Turn the protection enum into the native flags on non-Windows platforms
    #[cfg(not(target_os = "windows"))]
    pub fn into_native_flags(self) -> libc::c_int {
        match self {
            Self::ReadWrite => PROT_READ | PROT_WRITE,
            Self::ReadWriteExec => PROT_READ | PROT_WRITE | PROT_EXEC,
            Self::NoAccess => PROT_NONE,
        }
    }

    /// Turn the protection enum into the native flags on Windows platforms
    #[cfg(target_os = "windows")]
    pub fn into_native_flags(self) -> u32 {
        use windows_sys::Win32::System::Memory::*;
        match self {
            Self::ReadWrite => PAGE_READWRITE,
            Self::ReadWriteExec => PAGE_EXECUTE_READWRITE,
            Self::NoAccess => PAGE_NOACCESS,
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
macro_rules! mmap_anno_test_unused {
    () => {
        &$crate::util::os::MmapAnnotation::Test {
            file: file!(),
            line: line!(),
        }
    };
}

// Export this to external crates
pub use mmap_anno_test_unused;

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

/// Check the result from an mmap function in this module.
/// Return true if the mmap has failed due to an existing conflicting mapping.
pub(crate) fn result_is_mapped(result: Result<()>) -> bool {
    #[cfg(not(target_os = "windows"))]
    match result {
        Ok(_) => false,
        Err(err) => err.raw_os_error().unwrap() == libc::EEXIST,
    }
    #[cfg(target_os = "windows")]
    match result {
        Ok(_) => false,
        Err(err) => {
            // ERROR_INVALID_ADDRESS may be returned if the address is already mapped or invalid
            err.raw_os_error().unwrap()
                == windows_sys::Win32::Foundation::ERROR_INVALID_ADDRESS as i32
        }
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
    anno: &MmapAnnotation,
) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    #[cfg(target_os = "windows")]
    let flags = 0; // Not used
    let ret = mmap_fixed(start, size, flags, strategy, anno);
    // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)
    // On Windows, MEM_COMMIT guarantees zero-initialized pages.
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
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
    anno: &MmapAnnotation,
) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    let flags = MMAP_FLAGS;
    #[cfg(target_os = "windows")]
    let flags = 0; // Not used

    let ret = mmap_fixed(start, size, flags, strategy, anno);
    // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)
    // On Windows, MEM_COMMIT guarantees zero-initialized pages.
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
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
    anno: &MmapAnnotation,
) -> Result<()> {
    strategy.prot = MmapProtection::NoAccess;
    #[cfg(not(target_os = "windows"))]
    let flags = MMAP_FLAGS | libc::MAP_NORESERVE;
    #[cfg(target_os = "windows")]
    let flags = MAP_NORESERVE;
    mmap_fixed(start, size, flags, strategy, anno)
}

fn mmap_fixed(
    start: Address,
    size: usize,
    flags: libc::c_int,
    strategy: MmapStrategy,
    _anno: &MmapAnnotation,
) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    {
        let ptr = start.to_mut_ptr();
        let prot = strategy.prot.into_native_flags();
        wrap_libc_call(
            &|| unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) },
            ptr,
        )?;

        #[cfg(all(
            any(target_os = "linux", target_os = "android"),
            not(feature = "no_mmap_annotation")
        ))]
        {
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

    #[cfg(target_os = "windows")]
    {
        use std::io;
        use windows_sys::Win32::System::Memory::{
            VirtualAlloc, VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_FREE, MEM_RESERVE,
        };

        let ptr: *mut u8 = start.to_mut_ptr();
        let prot = strategy.prot.into_native_flags();

        // Has to COMMIT inmediately if:
        // - not MAP_NORESERVE
        // - and protection is not NoAccess
        let commit =
            (flags & MAP_NORESERVE) == 0 && !matches!(strategy.prot, MmapProtection::NoAccess);

        // Scan the region [ptr, ptr + size) to understand its current state
        unsafe {
            let mut addr = ptr;
            let end = ptr.add(size);

            let mut saw_free = false;
            let mut saw_reserved = false;
            let mut saw_committed = false;

            while addr < end {
                let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
                let q = VirtualQuery(
                    addr as *const _,
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                );
                if q == 0 {
                    return Err(io::Error::last_os_error());
                }

                let region_base = mbi.BaseAddress as *mut u8;
                let region_size = mbi.RegionSize;
                let region_end = region_base.add(region_size);

                // Calculate the intersection of [addr, end) and [region_base, region_end)
                let _sub_begin = if addr > region_base {
                    addr
                } else {
                    region_base
                };
                let _sub_end = if end < region_end { end } else { region_end };

                match mbi.State {
                    MEM_FREE => saw_free = true,
                    MEM_RESERVE => saw_reserved = true,
                    MEM_COMMIT => saw_committed = true,
                    _ => {
                        return Err(io::Error::other("Unexpected memory state in mmap_fixed"));
                    }
                }

                // Jump to the next region (VirtualQuery always returns "continuous regions with the same attributes")
                addr = region_end;
            }

            // 1. All FREE: make a new mapping in the region
            // 2. All RESERVE/COMMIT: treat as an existing mapping, can just COMMIT or succeed directly
            // 3. MIX of FREE + others: not allowed (semantically similar to MAP_FIXED_NOREPLACE)
            if saw_free && (saw_reserved || saw_committed) {
                return Err(io::Error::from_raw_os_error(
                    windows_sys::Win32::Foundation::ERROR_INVALID_ADDRESS as i32,
                ));
            }

            if saw_free && !saw_reserved && !saw_committed {
                // All FREE: make a new mapping in the region
                let mut allocation_type = MEM_RESERVE;
                if commit {
                    allocation_type |= MEM_COMMIT;
                }

                let res = VirtualAlloc(ptr as *mut _, size, allocation_type, prot);
                if res.is_null() {
                    return Err(io::Error::last_os_error());
                }

                Ok(())
            } else {
                // This behavior is similar to mmap with MAP_FIXED on Linux.
                // If the region is already mapped, we just ensure the required commitment.
                // If commit is not needed, we just return Ok.
                if commit {
                    let res = VirtualAlloc(ptr as *mut _, size, MEM_COMMIT, prot);
                    if res.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            }
        }
    }
}

/// Unmap the given memory (in page granularity). This wraps the unsafe libc munmap call.
pub fn munmap(start: Address, size: usize) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    return wrap_libc_call(&|| unsafe { libc::munmap(start.to_mut_ptr(), size) }, 0);

    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::Memory::*;
        // Using MEM_DECOMMIT will decommit the memory but leave the address space reserved.
        // This is the safest way to emulate munmap on Windows, as MEM_RELEASE would free
        // the entire allocation, which could be larger than the requested size.
        let res = unsafe { VirtualFree(start.to_mut_ptr(), size, MEM_DECOMMIT) };
        if res == 0 {
            // If decommit fails, we try to release the memory. This might happen if the memory was
            // only reserved.
            let res_release = unsafe { VirtualFree(start.to_mut_ptr(), 0, MEM_RELEASE) };
            if res_release == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }
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
                #[cfg(not(target_os = "windows"))]
                if os_errno == libc::ENOMEM {
                    // Signal `MmapOutOfMemory`. Expect the VM to abort immediately.
                    trace!("Signal MmapOutOfMemory!");
                    VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
                    unreachable!()
                }
                #[cfg(target_os = "windows")]
                if os_errno == windows_sys::Win32::Foundation::ERROR_NOT_ENOUGH_MEMORY as i32 {
                    // ERROR_NOT_ENOUGH_MEMORY
                    trace!("Signal MmapOutOfMemory!");
                    VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
                    unreachable!()
                }
            }
        }
        ErrorKind::AlreadyExists => {
            panic!("Failed to mmap, the address is already mapped. Should MMTk quarantine the address range first?");
        }
        _ => {
            #[cfg(target_os = "windows")]
            if let Some(os_errno) = error.raw_os_error() {
                // If it is invalid address, we provide a more specific panic message.
                if os_errno == windows_sys::Win32::Foundation::ERROR_INVALID_ADDRESS as i32 {
                    // ERROR_INVALID_ADDRESS
                    trace!("Signal MmapOutOfMemory!");
                    VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
                    unreachable!()
                }
            }
        }
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
pub(crate) fn panic_if_unmapped(_start: Address, _size: usize, _anno: &MmapAnnotation) {
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
    #[cfg(not(target_os = "windows"))]
    {
        let prot = prot.into_native_flags();
        wrap_libc_call(
            &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, prot) },
            0,
        )
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::Memory::*;
        let prot = prot.into_native_flags();
        let mut old_protect = 0;
        let res = unsafe { VirtualProtect(start.to_mut_ptr(), size, prot, &mut old_protect) };
        if res == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Protect the given memory (in page granularity) to forbid any access (PROT_NONE).
pub fn mprotect(start: Address, size: usize) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    {
        wrap_libc_call(
            &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_NONE) },
            0,
        )
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::Memory::*;
        let mut old_protect = 0;
        let res =
            unsafe { VirtualProtect(start.to_mut_ptr(), size, PAGE_NOACCESS, &mut old_protect) };
        if res == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
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
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    sys.total_memory()
}

#[cfg(test)]
mod tests {
    use crate::util::os::*;
    use crate::util::Address;
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
                        OSMemory::dzmmap(START, BYTES_IN_PAGE, MmapStrategy::TEST, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                    // We can overwrite with dzmmap
                    let res = unsafe {
                        OSMemory::dzmmap(START, BYTES_IN_PAGE, MmapStrategy { replace: true, ..MmapStrategy::TEST }, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE).is_ok());
                },
            );
        });
    }

    #[test]
    fn test_munmap() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = OSMemory::dzmmap(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy {
                            replace: false,
                            ..MmapStrategy::TEST
                        },
                        mmap_anno_test!(),
                    );
                    assert!(res.is_ok());
                    let res = OSMemory::munmap(START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE).is_ok());
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
                        OSMemory::dzmmap(START, BYTES_IN_PAGE, MmapStrategy::TEST, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                    // Use dzmmap_noreplace will fail
                    let res = OSMemory::dzmmap(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy {
                            replace: false,
                            ..MmapStrategy::TEST
                        },                        
                        mmap_anno_test!(),
                    );
                    assert!(res.is_err());
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE).is_ok());
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
                        OSMemory::dzmmap(START, BYTES_IN_PAGE, MmapStrategy { reserve: false, ..MmapStrategy::TEST }, mmap_anno_test!());
                    assert!(res.is_ok());
                    // Try reserve it
                    let res = unsafe {
                        OSMemory::dzmmap(START, BYTES_IN_PAGE, MmapStrategy { replace: true, ..MmapStrategy::TEST }, mmap_anno_test!())
                    };
                    assert!(res.is_ok());
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE).is_ok());
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
                    OSMemory::panic_if_unmapped(START, BYTES_IN_PAGE);
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[test]
    fn test_check_is_mmapped_for_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    assert!(OSMemory::dzmmap(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!()
                    )
                    .is_ok());
                    OSMemory::panic_if_unmapped(START, BYTES_IN_PAGE);
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE).is_ok());
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
                    assert!(OSMemory::dzmmap(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!(),
                    )
                    .is_ok());

                    // check if the next page is mapped - which should panic
                    OSMemory::panic_if_unmapped(START + BYTES_IN_PAGE, BYTES_IN_PAGE);
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE * 2).is_ok());
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
                    assert!(OSMemory::dzmmap(
                        START,
                        BYTES_IN_PAGE,
                        MmapStrategy::TEST,
                        mmap_anno_test!()
                    )
                    .is_ok());

                    // check if the 2 pages from START are mapped. The second page is unmapped, so it should panic.
                    OSMemory::panic_if_unmapped(START, BYTES_IN_PAGE * 2);
                },
                || {
                    assert!(OSMemory::munmap(START, BYTES_IN_PAGE * 2).is_ok());
                },
            )
        })
    }

    #[test]
    fn test_get_system_total_memory() {
        let total = OSMemory::get_system_total_memory().unwrap();
        println!("Total memory: {:?}", total);
    }
}
