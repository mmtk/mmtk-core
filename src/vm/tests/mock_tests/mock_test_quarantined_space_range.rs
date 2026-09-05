use super::mock_test_prelude::*;

use crate::policy::space::Space;
use crate::util::heap::layout::heap_parameters::MAX_SPACES;
use crate::util::heap::layout::vm_layout::VMLayout;
use crate::util::options::GCTriggerSelector;
use crate::util::os::*;
use crate::util::{Address, VMMutatorThread, VMThread};

#[test]
fn test_quarantined_space_range_semispace() {
    with_mockvm(
        default_setup,
        || {
            let default_layout = VMLayout::default();
            let dynamic_layout = VMLayout {
                dynamic_heap_range: true,
                ..default_layout
            };

            let normal_space_range_start = default_layout.heap_start;
            let normal_space_range_bytes = default_layout.max_space_extent();

            let reserved = OS::dzmmap(
                normal_space_range_start,
                normal_space_range_bytes,
                MmapStrategy::QUARANTINE,
                mmap_anno_test!(),
            )
            .expect("failed to quarantine one usual 64-bit space range");
            assert_eq!(reserved, normal_space_range_start);
            println!(
                "Occupy one normal space range [{}, {}) with quarantined memory",
                normal_space_range_start,
                normal_space_range_start + normal_space_range_bytes
            );
            println!(
                "Dynamic heap range is [{}, {})",
                unsafe { Address::from_usize(default_layout.max_space_extent()) },
                unsafe { Address::from_usize(1usize << VMLayout::LOG_ARCH_ADDRESS_SPACE) }
            );

            let mut builder = crate::mmtk::MMTKBuilder::new_no_env_vars();
            builder.set_vm_layout(dynamic_layout);
            builder
                .options
                .gc_trigger
                .set(GCTriggerSelector::FixedHeapSize(1024 * 1024));

            let mmtk = Box::leak(memory_manager::mmtk_init::<MockVM>(&builder));

            mmtk.get_plan()
                .for_each_space(&mut |space: &dyn Space<MockVM>| {
                    let common = space.common();
                    assert!(common.contiguous);
                    assert!(!common.descriptor.is_empty());

                    let descriptor = space.get_descriptor();
                    assert!(crate::mmtk::SFT_MAP.has_sft_entry(common.start));
                    assert!(descriptor.get_index() < MAX_SPACES);
                    assert!(descriptor.is_contiguous());
                    assert_eq!(descriptor.get_start(), common.start);
                    // descriptor.get_extent() stores 2TB, but common.extent stores the actual extent of the reserved range.
                    // So they are not equal. This can be confusing -- we should consider removing get_extent() from the descriptor, as they are not used anyway.
                    // assert_eq!(descriptor.get_extent(), common.extent);

                    let sft = crate::mmtk::SFT_MAP.get_checked(common.start);
                    assert_eq!(sft.name(), common.name);
                });

            memory_manager::initialize_collection(mmtk, VMThread::UNINITIALIZED);
            let mut mutator =
                memory_manager::bind_mutator(mmtk, VMMutatorThread(VMThread::UNINITIALIZED));
            let addr =
                memory_manager::alloc(&mut mutator, 8, 8, 0, crate::AllocationSemantics::Default);
            assert!(!addr.is_zero());
            let obj = MockVM::object_start_to_ref(addr);
            memory_manager::post_alloc(&mut mutator, obj, 8, crate::AllocationSemantics::Default);
            assert!(memory_manager::is_in_mmtk_spaces(obj));
        },
        no_cleanup,
    )
}
