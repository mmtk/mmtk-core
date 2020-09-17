use crate::plan::global::Plan;
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::policy::space::Space;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::vm::VMBinding;

use enum_map::EnumMap;

// This struct is part of the Mutator struct.
// We are trying to make it fixed-sized so that VM bindings can easily define a Mutator type to have the exact same layout as our Mutator struct.
#[repr(C)]
pub struct MutatorConfig<VM: VMBinding, P: Plan<VM> + 'static> {
    // Mapping between allocation semantics and allocator selector
    pub allocator_mapping: &'static EnumMap<AllocationType, AllocatorSelector>,
    // Mapping between allocator selector and spaces. Each pair represents a mapping.
    // Put this behind a box, so it is a pointer-sized field.
    #[allow(clippy::box_vec)]
    pub space_mapping: Box<Vec<(AllocatorSelector, &'static dyn Space<VM>)>>,
    // Plan-specific code for mutator prepare/release
    pub prepare_func: &'static dyn Fn(&mut Mutator<VM, P>, OpaquePointer),
    pub release_func: &'static dyn Fn(&mut Mutator<VM, P>, OpaquePointer),
}

// We are trying to make this struct fixed-sized so that VM bindings can easily define a type to have the exact same layout as this struct.
// Currently Mutator is fixed sized, and we should try keep this invariant:
// - Allocators are fixed-length arrays of allocators.
// - MutatorConfig has 3 pointers/refs (including one fat pointer), and is fixed sized.
#[repr(C)]
pub struct Mutator<VM: VMBinding, P: Plan<VM> + 'static> {
    pub allocators: Allocators<VM>,
    pub mutator_tls: OpaquePointer,
    pub plan: &'static P,
    pub config: MutatorConfig<VM, P>,
}

impl<VM: VMBinding, P: Plan<VM>> MutatorContext<VM> for Mutator<VM, P> {
    fn prepare(&mut self, tls: OpaquePointer) {
        (*self.config.prepare_func)(self, tls)
    }
    fn release(&mut self, tls: OpaquePointer) {
        (*self.config.release_func)(self, tls)
    }

    // Note that this method is slow, and we expect VM bindings that care about performance to implement allocation fastpath sequence in their bindings.
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address {
        unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        }
        .alloc(size, align, offset)
    }

    // Note that this method is slow, and we expect VM bindings that care about performance to implement allocation fastpath sequence in their bindings.
    // Q: Can we remove type_refer?
    fn post_alloc(
        &mut self,
        refer: ObjectReference,
        _type_refer: ObjectReference,
        _bytes: usize,
        allocator: AllocationType,
    ) {
        unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        }
        .get_space()
        .unwrap()
        .initialize_header(refer, true)
    }

    fn get_tls(&self) -> OpaquePointer {
        self.mutator_tls
    }
}

// TODO: We should be able to remove this trait, as we removed per-plan mutator implementation, and there is no other type that implements this trait.
// The Mutator struct above is the only type that implements this trait. We should be able to merge them.
pub trait MutatorContext<VM: VMBinding>: Send + Sync + 'static {
    fn prepare(&mut self, tls: OpaquePointer);
    fn release(&mut self, tls: OpaquePointer);
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address;
    fn post_alloc(
        &mut self,
        refer: ObjectReference,
        type_refer: ObjectReference,
        bytes: usize,
        allocator: AllocationType,
    );
    fn flush_remembered_sets(&mut self) {}
    fn get_tls(&self) -> OpaquePointer;
}
