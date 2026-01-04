use crate::util::os::memory::*;
use crate::util::address::Address;
use std::io::Result;

pub fn posix_mmap(start: Address, size: usize, strategy: MmapStrategy, annotation: &MmapAnnotation<'_>) -> Result<Address> {
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

pub fn posix_panic_if_unmapped(start: Address, size: usize, anno: &MmapAnnotation) {
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

fn wrap_libc_call<T: PartialEq>(f: &dyn Fn() -> T, expect: T) -> Result<()> {
    let ret = f();
    if ret == expect {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}