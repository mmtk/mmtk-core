// GITHUB-CI: MMTK_PLAN=all

use mmtk::util::conversions::*;
use mmtk::util::heap::vm_layout::VMLayout;
use mmtk::util::Address;

use crate::tests::vm_layout_default::test_with_vm_layout;

// This test only run on 64bits.

#[test]
fn test_vm_layout_compressed_pointer() {
    let start = if cfg!(target_os = "macos") {
        // Impossible to map 0x4000_0000 on maocOS. SO choose a different address.
        0x40_0000_0000
    } else {
        0x4000_0000
    };
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
}
