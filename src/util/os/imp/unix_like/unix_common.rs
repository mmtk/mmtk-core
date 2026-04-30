use crate::util::address::Address;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::conversions::raw_align_up;
use crate::util::os::*;
use std::io::Result;

impl MmapProtection {
    fn get_native_flags(&self) -> i32 {
        use libc::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
        match self {
            Self::ReadWrite => PROT_READ | PROT_WRITE,
            Self::ReadWriteExec => PROT_READ | PROT_WRITE | PROT_EXEC,
            Self::NoAccess => PROT_NONE,
        }
    }
}

pub fn mmap(
    start: Address,
    size: usize,
    strategy: MmapStrategy,
    annotation: &MmapAnnotation<'_>,
) -> MmapResult<Address> {
    let ptr = start.to_mut_ptr();
    let prot = strategy.prot.get_native_flags();
    let flags = strategy.get_posix_mmap_flags(true);
    wrap_libc_call(
        &|| unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) },
        ptr,
    )
    .map_err(|e| MmapError::new(start, size, annotation, e))?;
    Ok(start)
}

pub fn mmap_anywhere(size: usize, align: usize, strategy: MmapStrategy) -> Result<Address> {
    let (aligned_start, leading, trailing) =
        mmap_aligned(std::ptr::null_mut(), size, align, strategy)?;
    trim_unaligned_mmap_edges(aligned_start, leading, trailing).map_err(|e| e.error)?;
    Ok(aligned_start)
}

pub fn mmap_preferred(
    start: Address,
    size: usize,
    align: usize,
    strategy: MmapStrategy,
    annotation: &MmapAnnotation<'_>,
) -> MmapResult<Address> {
    let (aligned_start, leading, trailing) =
        mmap_aligned(start.to_mut_ptr(), size, align, strategy)
            .map_err(|e| MmapError::new(start, raw_align_up(size, align) + align, annotation, e))?;
    trim_unaligned_mmap_edges(aligned_start, leading, trailing)
        .map_err(|e| MmapError::new(e.start, e.size, annotation, e.error))?;
    Ok(aligned_start)
}

fn mmap_aligned(
    preferred_start: *mut libc::c_void,
    size: usize,
    align: usize,
    strategy: MmapStrategy,
) -> Result<(Address, Option<(Address, usize)>, Option<(Address, usize)>)> {
    debug_assert!(align.is_power_of_two());
    debug_assert!(align % BYTES_IN_PAGE == 0);
    debug_assert!(size % BYTES_IN_PAGE == 0);

    let aligned_size = raw_align_up(size, align);
    let alloc_size = aligned_size + align;
    let prot = strategy.prot.get_native_flags();
    let flags = strategy.get_posix_mmap_flags(false);

    let ptr = unsafe { libc::mmap(preferred_start, alloc_size, prot, flags, -1, 0) };
    if ptr == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error());
    }

    let start = Address::from_mut_ptr(ptr);
    let aligned_start = start.align_up(align);

    let leading_unaligned_size = aligned_start - start;
    let trailing_unaligned_size = alloc_size - leading_unaligned_size - size;

    let leading = (leading_unaligned_size > 0).then_some((start, leading_unaligned_size));
    let trailing =
        (trailing_unaligned_size > 0).then_some((aligned_start + size, trailing_unaligned_size));

    Ok((aligned_start, leading, trailing))
}

struct MunmapFailure {
    start: Address,
    size: usize,
    error: std::io::Error,
}

fn trim_unaligned_mmap_edges(
    _aligned_start: Address,
    leading: Option<(Address, usize)>,
    trailing: Option<(Address, usize)>,
) -> std::result::Result<(), MunmapFailure> {
    if let Some((start, size)) = leading {
        debug_assert!(size % BYTES_IN_PAGE == 0);
        munmap(start, size).map_err(|error| MunmapFailure { start, size, error })?;
    }

    if let Some((start, size)) = trailing {
        debug_assert!(size % BYTES_IN_PAGE == 0);
        munmap(start, size).map_err(|error| MunmapFailure { start, size, error })?;
    }

    Ok(())
}

pub fn is_mmap_oom(os_errno: i32) -> bool {
    os_errno == libc::ENOMEM
}

pub fn munmap(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(&|| unsafe { libc::munmap(start.to_mut_ptr(), size) }, 0)
}

pub fn mprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, prot.get_native_flags()) },
        0,
    )
}

pub type ProcessIDType = libc::pid_t;
pub type ThreadIDType = libc::pthread_t;

pub fn get_process_id() -> Result<ProcessIDType> {
    Ok(unsafe { libc::getpid() })
}

pub fn get_thread_id() -> Result<ThreadIDType> {
    Ok(unsafe { libc::pthread_self() })
}

pub fn wrap_libc_call<T: PartialEq>(f: &dyn Fn() -> T, expect: T) -> Result<()> {
    let ret = f();
    if ret == expect {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
    use crate::util::test_util::{serial_test, with_cleanup};
    use std::io::ErrorKind;

    fn assert_mapping_state(start: Address, size: usize, expect_mapped: bool) {
        let annotation = MmapAnnotation::Misc {
            name: "mmap_anywhere_test",
        };
        match mmap(
            start,
            size,
            MmapStrategy::QUARANTINE.replace(false),
            &annotation,
        ) {
            Ok(_) => {
                let _ = munmap(start, size);
                assert!(
                    !expect_mapped,
                    "{start} of size {size} should still be mapped"
                );
            }
            Err(e) => {
                assert_eq!(e.error.kind(), ErrorKind::AlreadyExists);
                assert!(expect_mapped, "{start} of size {size} should be unmapped");
            }
        }
    }

    #[test]
    fn mmap_anywhere_unmaps_alignment_padding() {
        serial_test(|| {
            let size = BYTES_IN_CHUNK + BYTES_IN_PAGE;
            let start = mmap_anywhere(size, BYTES_IN_CHUNK, MmapStrategy::QUARANTINE).unwrap();

            with_cleanup(
                || {
                    assert!(start.is_aligned_to(BYTES_IN_CHUNK));
                    assert_mapping_state(start + size - BYTES_IN_PAGE, BYTES_IN_PAGE, true);
                    assert_mapping_state(start + size, BYTES_IN_PAGE, false);
                },
                || {
                    let _ = munmap(start, size);
                },
            );
        });
    }
}
