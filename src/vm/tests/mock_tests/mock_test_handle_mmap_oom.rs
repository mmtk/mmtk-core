use super::mock_test_prelude::*;

use crate::util::memory;
use crate::util::opaque_pointer::*;
use crate::util::Address;

#[cfg(target_pointer_width = "32")]
const LARGE_SIZE: usize = 4_294_967_295;
#[cfg(target_pointer_width = "64")]
const LARGE_SIZE: usize = 1_000_000_000_000;

#[test]
pub fn test_handle_mmap_oom() {
    with_mockvm(
        default_setup,
        || {
            let panic_res = std::panic::catch_unwind(move || {
                let start = unsafe { Address::from_usize(0x100_0000) };
                // mmap 1 terabyte memory - we expect this will fail due to out of memory.
                // If that's not the case, increase the size we mmap.
                let mmap_res =
                    memory::dzmmap_noreplace(start, LARGE_SIZE, memory::MmapStrategy::Normal);

                memory::handle_mmap_error::<MockVM>(
                    mmap_res.err().unwrap(),
                    VMThread::UNINITIALIZED,
                );
            });
            assert!(panic_res.is_err());

            // The error should match the default implementation of Collection::out_of_memory()
            let err = panic_res.err().unwrap();
            assert!(err.is::<String>());
            assert_eq!(
                err.downcast_ref::<String>().unwrap(),
                &"Out of memory with MmapOutOfMemory!"
            );
        },
        || {
            read_mockvm(|mock| {
                assert!(mock.out_of_memory.is_called());
            })
        },
    )
}
