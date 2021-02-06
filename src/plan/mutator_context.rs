use crate::plan::barriers::{Barrier, WriteTarget};
use crate::plan::global::Plan;
use crate::plan::AllocationSemantics as AllocationType;
use crate::policy::space::Space;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

use enum_map::EnumMap;

type SpaceMapping<VM> = Vec<(AllocatorSelector, &'static dyn Space<VM>)>;

// This struct is part of the Mutator struct.
// We are trying to make it fixed-sized so that VM bindings can easily define a Mutator type to have the exact same layout as our Mutator struct.
#[repr(C)]
pub struct MutatorConfig<VM: VMBinding> {
    // Mapping between allocation semantics and allocator selector
    pub allocator_mapping: &'static EnumMap<AllocationType, AllocatorSelector>,
    // Mapping between allocator selector and spaces. Each pair represents a mapping.
    // Put this behind a box, so it is a pointer-sized field.
    #[allow(clippy::box_vec)]
    pub space_mapping: Box<SpaceMapping<VM>>,
    // Plan-specific code for mutator prepare/release
    pub prepare_func: &'static dyn Fn(&mut Mutator<VM>, OpaquePointer),
    pub release_func: &'static dyn Fn(&mut Mutator<VM>, OpaquePointer),
}

unsafe impl<VM: VMBinding> Send for MutatorConfig<VM> {}
unsafe impl<VM: VMBinding> Sync for MutatorConfig<VM> {}

/// A mutator is a per-thread data structure that manages allocations and barriers. It is usually highly coupled with the language VM.
/// It is recommended for MMTk users 1) to have a mutator struct of the same layout in the thread local storage that can be accessed efficiently,
/// and 2) to implement fastpath allocation and barriers for the mutator in the VM side.

// We are trying to make this struct fixed-sized so that VM bindings can easily define a type to have the exact same layout as this struct.
// Currently Mutator is fixed sized, and we should try keep this invariant:
// - Allocators are fixed-length arrays of allocators.
// - MutatorConfig only has pointers/refs (including fat pointers), and is fixed sized.
#[repr(C)]
pub struct Mutator<VM: VMBinding> {
    pub allocators: Allocators<VM>,
    pub barrier: Box<dyn Barrier>,
    pub mutator_tls: OpaquePointer,
    pub plan: &'static dyn Plan<VM = VM>,
    pub config: MutatorConfig<VM>,
}

impl<VM: VMBinding> MutatorContext<VM> for Mutator<VM> {
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
    fn post_alloc(&mut self, refer: ObjectReference, _bytes: usize, allocator: AllocationType) {
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

    fn barrier(&mut self) -> &mut dyn Barrier {
        &mut *self.barrier
    }
}

/// Each GC plan should provide their implementation of a MutatorContext. *Note that this trait is no longer needed as we removed
/// per-plan mutator implementation and we will remove this trait as well in the future.*

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
    fn post_alloc(&mut self, refer: ObjectReference, bytes: usize, allocator: AllocationType);
    fn flush_remembered_sets(&mut self) {
        self.barrier().flush();
    }
    fn flush(&mut self) {
        self.flush_remembered_sets();
    }
    fn get_tls(&self) -> OpaquePointer;
    fn barrier(&mut self) -> &mut dyn Barrier;

    fn record_modified_node(&mut self, obj: ObjectReference) {
        self.barrier().post_write_barrier(WriteTarget::Object(obj));
    }
    fn record_modified_edge(&mut self, slot: Address) {
        self.barrier().post_write_barrier(WriteTarget::Slot(slot));
    }
}
