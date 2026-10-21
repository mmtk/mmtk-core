// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;
use super::mock_test_vm_layout_default::test_with_vm_layout;
use crate::util::conversions::*;
use crate::util::heap::vm_layout::VMLayout;
use crate::util::Address;

// This test only run on 64bits.

#[test]
fn test_vm_layout_compressed_pointer() {
    with_mockvm(
        default_setup,
        || {
            #[cfg(target_os = "macos")]
            let start = reserve_heap_start_on_macos();
            #[cfg(not(target_os = "macos"))]
            let start = 0x4000_0000;
            let heap_size = 1024 * 1024;
            let end = match start + heap_size {
                end if end <= (4usize << 30) => 4usize << 30,
                end if end <= (32usize << 30) => 32usize << 30,
                _ => start + (32usize << 30),
            };
            let layout = VMLayout {
                log_address_space: 35,
                heap_start: chunk_align_down(unsafe { Address::from_usize(start) }),
                heap_end: chunk_align_up(unsafe { Address::from_usize(end) }),
                log_space_extent: 31,
                force_use_contiguous_spaces: false,
                dynamic_heap_range: false,
            };
            test_with_vm_layout(Some(layout));
        },
        no_cleanup,
    )
}

/// Impossible to map 0x4000_0000 on macOS, so we need a different address below 32GB. But macOS cannot mmap
/// at a fixed address without replacing existing mappings, and on aarch64 the OS places its own allocations
/// (e.g. MMTk's own SFT map, allocated with malloc during initialization) anywhere in the range below 32GB.
/// If one lands where the heap starts, MMTk silently replaces and zeroes it.
///
/// So we let the OS pick a free range, and keep it reserved for the rest of the process so nothing else is
/// placed there. MMTk later maps its heap over our reservation, which is fine as we own it.
#[cfg(target_os = "macos")]
fn reserve_heap_start_on_macos() -> usize {
    use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
    // Every plan only maps the first chunk for this test. Reserve a lot more than that.
    const RESERVE_BYTES: usize = 64 << 20;
    // Reserve an extra chunk so we can align the start to a chunk.
    let res = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            RESERVE_BYTES + BYTES_IN_CHUNK,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE,
            -1,
            0,
        )
    };
    assert_ne!(res, libc::MAP_FAILED);
    let start = raw_align_up(res as usize, BYTES_IN_CHUNK);
    assert!(
        start + RESERVE_BYTES <= 32usize << 30,
        "The OS reserved memory at {start:#x}, which is not below 32GB"
    );
    start
}
