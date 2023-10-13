// GITHUB-CI: MMTK_PLAN=NoGC,SemiSpace,Immix,GenImmix,StickyImmix

use crate::test_fixtures::{MMTKSingleton, SerialFixture};
use crate::DummyVM;

use mmtk::util::alloc::Allocator;
use mmtk::util::alloc::BumpAllocator;
use mmtk::util::Address;
use mmtk::util::OpaquePointer;
use mmtk::util::{VMMutatorThread, VMThread};
use mmtk::AllocationSemantics;

lazy_static! {
    static ref MMTK_SINGLETON: SerialFixture<MMTKSingleton> = SerialFixture::new();
}

#[test]
pub fn acquire_typed_allocator() {
    MMTK_SINGLETON.with_fixture(|fixture| {
        let tls_opaque_pointer = VMMutatorThread(VMThread(OpaquePointer::UNINITIALIZED));
        static mut DEFAULT_ALLOCATOR_OFFSET: usize = 0;

        // ANCHOR: avoid_resolving_allocator
        // At boot time
        let selector = mmtk::memory_manager::get_allocator_mapping(
            &fixture.mmtk,
            AllocationSemantics::Default,
        );
        unsafe {
            DEFAULT_ALLOCATOR_OFFSET =
                mmtk::plan::Mutator::<DummyVM>::get_allocator_base_offset(selector);
        }
        let mutator = mmtk::memory_manager::bind_mutator(&fixture.mmtk, tls_opaque_pointer);

        // At run time: allocate with the default semantics without resolving allocator
        let default_allocator: &mut BumpAllocator<DummyVM> = {
            let mutator_addr = Address::from_ref(&*mutator);
            unsafe {
                (mutator_addr + DEFAULT_ALLOCATOR_OFFSET).as_mut_ref::<BumpAllocator<DummyVM>>()
            }
        };
        let addr = default_allocator.alloc(8, 8, 0);
        // ANCHOR_END: avoid_resolving_allocator

        assert!(!addr.is_zero());
    });
}
