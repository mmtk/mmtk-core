use super::mock_test_prelude::*;

use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::util::{VMMutatorThread, VMThread};
use crate::AllocationSemantics;

lazy_static! {
    static ref FIXTURE: Fixture<MMTKFixture> = Fixture::new();
}

#[test]
pub fn acquire_typed_allocator() {
    with_mockvm(
        default_setup,
        || {
            let fixture = MMTKFixture::create();
            let tls_opaque_pointer = VMMutatorThread(VMThread(OpaquePointer::UNINITIALIZED));
            static mut DEFAULT_ALLOCATOR_OFFSET: usize = 0;

            // ANCHOR: avoid_resolving_allocator
            // At boot time
            let selector =
                memory_manager::get_allocator_mapping(fixture.mmtk, AllocationSemantics::Default);
            unsafe {
                DEFAULT_ALLOCATOR_OFFSET =
                    crate::plan::Mutator::<MockVM>::get_allocator_base_offset(selector);
            }
            let mutator = memory_manager::bind_mutator(fixture.mmtk, tls_opaque_pointer);

            // At run time: allocate with the default semantics without resolving allocator
            let default_allocator: &mut BumpAllocator<MockVM> = {
                let mutator_addr = Address::from_ref(&*mutator);
                unsafe {
                    (mutator_addr + DEFAULT_ALLOCATOR_OFFSET).as_mut_ref::<BumpAllocator<MockVM>>()
                }
            };
            let addr = default_allocator.alloc(8, 8, 0);
            // ANCHOR_END: avoid_resolving_allocator

            assert!(!addr.is_zero());
        },
        no_cleanup,
    )
}
