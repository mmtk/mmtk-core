use super::mock_test_prelude::*;

use crate::util::opaque_pointer::*;
use crate::util::os::*;
use crate::util::Address;

#[test]
pub fn test_handle_mmap_conflict() {
    with_mockvm(
        default_setup,
        || {
            let start = unsafe { Address::from_usize(0x100_0000) };
            let one_megabyte = 1000000;
            let mmap1_res = OS::dzmmap(start, one_megabyte, MmapStrategy::TEST, mmap_anno_test!());
            assert!(mmap1_res.is_ok());

            let panic_res = std::panic::catch_unwind(|| {
                let mmap2_res = OS::dzmmap(
                    start,
                    one_megabyte,
                    MmapStrategy {
                        replace: false,
                        ..MmapStrategy::TEST
                    },
                    mmap_anno_test!(),
                );
                assert!(mmap2_res.is_err());
                OS::handle_mmap_error::<MockVM>(
                    mmap2_res.err().unwrap(),
                    VMThread::UNINITIALIZED,
                    start,
                    one_megabyte,
                );
            });

            // The error should match the error message in memory::handle_mmap_error()
            assert!(panic_res.is_err());
            let err = panic_res.err().unwrap();
            assert!(err.is::<&str>());
            assert_eq!(err.downcast_ref::<&str>().unwrap(), &"Failed to mmap, the address is already mapped. Should MMTk quarantine the address range first?");
        },
        no_cleanup,
    )
}
