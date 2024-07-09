// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=is_mmtk_object

use super::mock_test_prelude::*;

use crate::util::*;
use crate::vm::ObjectModel;
use crate::AllocationSemantics;

#[test]
pub fn interior_poiner_in_normal_object() {
    const MB: usize = 1024 * 1024;
    const OBJECT_SIZE: usize = 16;
    with_mockvm(
        || -> MockVM {
            MockVM {
                get_object_size: MockMethod::new_fixed(Box::new(|_| OBJECT_SIZE)),
                ..MockVM::default()
            }
        },
        || {
            let mut fixture = MutatorFixture::create_with_heapsize(10 * MB);

            let mut test_obj = || {
                let addr = memory_manager::alloc(
                    &mut fixture.mutator,
                    OBJECT_SIZE,
                    8,
                    0,
                    AllocationSemantics::Default,
                );
                assert!(!addr.is_zero());

                let obj = MockVM::address_to_ref(addr);
                println!(
                    "start = {}, end = {}, obj = {}, in-obj addr = {}",
                    addr,
                    addr + OBJECT_SIZE,
                    obj,
                    obj.to_address::<MockVM>()
                );
                memory_manager::post_alloc(
                    &mut fixture.mutator,
                    obj,
                    OBJECT_SIZE,
                    AllocationSemantics::Default,
                );

                let test_internal_ptr = |ptr: Address| {
                    if ptr >= addr + OBJECT_SIZE {
                        println!("ptr = {}, not internal pointer", ptr);
                        // not internal pointer
                        let base_ref = crate::memory_manager::find_object_from_internal_pointer::<
                            MockVM,
                        >(ptr, usize::MAX);
                        println!("{:?}", base_ref);
                        assert!(base_ref.is_none());
                    } else {
                        println!("ptr = {}, internal pointer", ptr);
                        // is internal pointer
                        let base_ref = crate::memory_manager::find_object_from_internal_pointer::<
                            MockVM,
                        >(ptr, usize::MAX);
                        assert!(base_ref.is_some());
                        assert_eq!(base_ref.unwrap(), obj);
                    }
                };

                // Test with the first 16 bytes as offset in the object
                for offset in 0..16usize {
                    let ptr = obj.to_raw_address() + offset;
                    test_internal_ptr(ptr);
                }

                // Test with the first 16 bytes after the object size
                for offset in OBJECT_SIZE..(OBJECT_SIZE + 16) {
                    let ptr = obj.to_raw_address() + offset;
                    test_internal_ptr(ptr);
                }
            };

            test_obj();
        },
        no_cleanup,
    )
}
