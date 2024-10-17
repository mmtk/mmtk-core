// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;
use super::mock_test_vm_layout_default::test_with_vm_layout;
use crate::util::conversions::*;
use crate::util::heap::vm_layout::VMLayout;
use crate::util::heap::vm_layout::BYTES_IN_CHUNK;
use crate::util::Address;

// This test only run on 64bits.

#[test]
fn test_vm_layout_compressed_pointer() {
    with_mockvm(
        default_setup,
        || {
            let heap_size = 1024 * 1024;
            let start = crate::util::memory::find_usable_address(heap_size, BYTES_IN_CHUNK)
                .expect("Cannot find usable address range");
            println!("Use {} as heap start", start);
            let end = match start.as_usize() + heap_size {
                end if end <= (4usize << 30) => 4usize << 30,
                end if end <= (32usize << 30) => 32usize << 30,
                _ => start.as_usize() + (32usize << 30),
            };
            let layout = VMLayout {
                log_address_space: 35,
                heap_start: start,
                heap_end: chunk_align_up(unsafe { Address::from_usize(end) }),
                log_space_extent: 31,
                force_use_contiguous_spaces: false,
            };
            test_with_vm_layout(Some(layout));
        },
        no_cleanup,
    )
}
