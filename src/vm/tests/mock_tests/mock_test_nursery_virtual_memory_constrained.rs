// GITHUB-CI: MMTK_PLAN=GenImmix

use super::mock_test_prelude::*;

use crate::util::heap::layout::vm_layout::*;
use crate::util::options::*;

#[test]
pub fn constrained() {
    with_mockvm(
        default_setup,
        || {
            // 4G heap with a 32 bits vm layout.
            const MB: usize = 1024 * 1024;
            let heap_size = 4096 * MB;
            let fixture = MutatorFixture::create_with_builder(|builder| {
                builder.set_vm_layout(VMLayout::new_32bit());
                builder
                    .options
                    .nursery
                    .set(NurserySize::ProportionalBounded {
                        min: 0.1f64,
                        max: 1.0f64,
                    });
                builder
                    .options
                    .gc_trigger
                    .set(GCTriggerSelector::FixedHeapSize(heap_size));
            });
            // Use the default 32MB virtual memory.
            assert_eq!(
                fixture
                    .mmtk()
                    .get_options()
                    .nursery
                    .estimate_virtual_memory_in_bytes(heap_size),
                32 * MB,
            );
        },
        no_cleanup,
    )
}
