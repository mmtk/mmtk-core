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
            };
            test_with_vm_layout(Some(layout));
        },
        no_cleanup,
    )
}

/// Impossible to map 0x4000_0000 on macOS, so we need a different address below 32GB. But macOS cannot mmap
/// at a fixed address without replacing existing mappings, and on aarch64 the OS places large allocations
/// (e.g. MMTk's own SFT map, allocated with malloc during initialization) in the same range above the dyld
/// shared cache. If one lands where the heap starts, MMTk silently replaces and zeroes it.
///
/// So we reserve the start of the heap before MMTk initializes, and keep the reservation for the rest of
/// the process. MMTk later maps its heap over our reservation, which is fine as we own it.
#[cfg(target_os = "macos")]
fn reserve_heap_start_on_macos() -> usize {
    use crate::util::test_util::{
        CHUNK_STATE_MMAPPER_TEST_REGION, RAW_MEMORY_FREELIST_TEST_REGION,
    };
    // Every plan only maps the first chunk for this test. Reserve a lot more than that.
    const RESERVE_BYTES: usize = 64 << 20;
    // Other tests mmap at fixed addresses in this range. Avoid it in case they run in the same process.
    let other_tests = RAW_MEMORY_FREELIST_TEST_REGION.start.as_usize()
        ..(CHUNK_STATE_MMAPPER_TEST_REGION.start + CHUNK_STATE_MMAPPER_TEST_REGION.size).as_usize();
    // Try from 12GB (right above the dyld shared cache on aarch64), while staying below 32GB.
    // The OS's own large allocations may take any part of this range, so we try every candidate.
    for candidate in (0x3_0000_0000usize..(32usize << 30) - RESERVE_BYTES).step_by(RESERVE_BYTES) {
        if candidate < other_tests.end && candidate + RESERVE_BYTES > other_tests.start {
            continue;
        }
        // Without MAP_FIXED, the address is only a hint. The OS gives us a different address
        // if anything is already mapped in the range.
        let res = unsafe {
            libc::mmap(
                candidate as *mut libc::c_void,
                RESERVE_BYTES,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if res as usize == candidate {
            return candidate;
        }
        if res != libc::MAP_FAILED {
            unsafe { libc::munmap(res, RESERVE_BYTES) };
        }
    }
    panic!("Failed to reserve a heap range below 32GB");
}
