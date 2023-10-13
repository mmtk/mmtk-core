//! Mutator context for each application thread.

use crate::plan::barriers::Barrier;
use crate::plan::global::Plan;
use crate::plan::AllocationSemantics;
use crate::policy::space::Space;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::Allocator;
use crate::util::{Address, ObjectReference};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;

use enum_map::EnumMap;

pub(crate) type SpaceMapping<VM> = Vec<(AllocatorSelector, &'static dyn Space<VM>)>;

/// A place-holder implementation for `MutatorConfig::prepare_func` that should not be called.
/// It is the most often used by plans that sets `PlanConstraints::needs_prepare_mutator` to
/// `false`.  It is also used by `NoGC` because it must not trigger GC.
pub(crate) fn unreachable_prepare_func<VM: VMBinding>(
    _mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    unreachable!("`MutatorConfig::prepare_func` must not be called for the current plan.")
}

/// A place-holder implementation for `MutatorConfig::release_func` that should not be called.
/// Currently only used by `NoGC`.
pub(crate) fn unreachable_release_func<VM: VMBinding>(
    _mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    unreachable!("`MutatorConfig::release_func` must not be called for the current plan.")
}

/// A place-holder implementation for `MutatorConfig::release_func` that does nothing.
pub(crate) fn no_op_release_func<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

// This struct is part of the Mutator struct.
// We are trying to make it fixed-sized so that VM bindings can easily define a Mutator type to have the exact same layout as our Mutator struct.
#[repr(C)]
pub struct MutatorConfig<VM: VMBinding> {
    /// Mapping between allocation semantics and allocator selector
    pub allocator_mapping: &'static EnumMap<AllocationSemantics, AllocatorSelector>,
    /// Mapping between allocator selector and spaces. Each pair represents a mapping.
    /// Put this behind a box, so it is a pointer-sized field.
    #[allow(clippy::box_collection)]
    pub space_mapping: Box<SpaceMapping<VM>>,
    /// Plan-specific code for mutator prepare. The VMWorkerThread is the worker thread that executes this prepare function.
    pub prepare_func: &'static (dyn Fn(&mut Mutator<VM>, VMWorkerThread) + Send + Sync),
    /// Plan-specific code for mutator release. The VMWorkerThread is the worker thread that executes this release function.
    pub release_func: &'static (dyn Fn(&mut Mutator<VM>, VMWorkerThread) + Send + Sync),
}

impl<VM: VMBinding> std::fmt::Debug for MutatorConfig<VM> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MutatorConfig:\n")?;
        f.write_str("Semantics mapping:\n")?;
        for (semantic, selector) in self.allocator_mapping.iter() {
            let space_name: &str = match self
                .space_mapping
                .iter()
                .find(|(selector_to_find, _)| selector_to_find == selector)
            {
                Some((_, space)) => space.name(),
                None => "!!!missing space here!!!",
            };
            f.write_fmt(format_args!(
                "- {:?} = {:?} ({:?})\n",
                semantic, selector, space_name
            ))?;
        }
        f.write_str("Space mapping:\n")?;
        for (selector, space) in self.space_mapping.iter() {
            f.write_fmt(format_args!("- {:?} = {:?}\n", selector, space.name()))?;
        }
        Ok(())
    }
}

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
    pub barrier: Box<dyn Barrier<VM>>,
    /// The mutator thread that is bound with this Mutator struct.
    pub mutator_tls: VMMutatorThread,
    pub plan: &'static dyn Plan<VM = VM>,
    pub config: MutatorConfig<VM>,
}

impl<VM: VMBinding> MutatorContext<VM> for Mutator<VM> {
    fn prepare(&mut self, tls: VMWorkerThread) {
        (*self.config.prepare_func)(self, tls)
    }
    fn release(&mut self, tls: VMWorkerThread) {
        (*self.config.release_func)(self, tls)
    }

    // Note that this method is slow, and we expect VM bindings that care about performance to implement allocation fastpath sequence in their bindings.
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
    ) -> Address {
        unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        }
        .alloc(size, align, offset)
    }

    fn alloc_slow(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
    ) -> Address {
        unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        }
        .alloc_slow(size, align, offset)
    }

    // Note that this method is slow, and we expect VM bindings that care about performance to implement allocation fastpath sequence in their bindings.
    fn post_alloc(
        &mut self,
        refer: ObjectReference,
        _bytes: usize,
        allocator: AllocationSemantics,
    ) {
        unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        }
        .get_space()
        .initialize_object_metadata(refer, true)
    }

    fn get_tls(&self) -> VMMutatorThread {
        self.mutator_tls
    }

    /// Used by specialized barrier slow-path calls to avoid dynamic dispatches.
    unsafe fn barrier_impl<B: Barrier<VM>>(&mut self) -> &mut B {
        debug_assert!(self.barrier().is::<B>());
        let (payload, _vptr) = std::mem::transmute::<_, (*mut B, *mut ())>(self.barrier());
        &mut *payload
    }

    fn barrier(&mut self) -> &mut dyn Barrier<VM> {
        &mut *self.barrier
    }
}

impl<VM: VMBinding> Mutator<VM> {
    /// Get all the valid allocator selector (no duplicate)
    fn get_all_allocator_selectors(&self) -> Vec<AllocatorSelector> {
        use itertools::Itertools;
        self.config
            .allocator_mapping
            .iter()
            .map(|(_, selector)| *selector)
            .sorted()
            .dedup()
            .filter(|selector| *selector != AllocatorSelector::None)
            .collect()
    }

    /// Inform each allocator about destroying. Call allocator-specific on destroy methods.
    pub fn on_destroy(&mut self) {
        for selector in self.get_all_allocator_selectors() {
            unsafe { self.allocators.get_allocator_mut(selector) }.on_mutator_destroy();
        }
    }

    /// Get the allocator for the selector.
    ///
    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    /// [`crate::memory_manager::get_allocator_mapping`] can be used to get a selector.
    pub unsafe fn allocator(&self, selector: AllocatorSelector) -> &dyn Allocator<VM> {
        self.allocators.get_allocator(selector)
    }

    /// Get the mutable allocator for the selector.
    ///
    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    /// [`crate::memory_manager::get_allocator_mapping`] can be used to get a selector.
    pub unsafe fn allocator_mut(&mut self, selector: AllocatorSelector) -> &mut dyn Allocator<VM> {
        self.allocators.get_allocator_mut(selector)
    }

    /// Get the allocator of a concrete type for the selector.
    ///
    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    /// [`crate::memory_manager::get_allocator_mapping`] can be used to get a selector.
    pub unsafe fn allocator_impl<T: Allocator<VM>>(&self, selector: AllocatorSelector) -> &T {
        self.allocators.get_typed_allocator(selector)
    }

    /// Get the mutable allocator of a concrete type for the selector.
    ///
    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    /// [`crate::memory_manager::get_allocator_mapping`] can be used to get a selector.
    pub unsafe fn allocator_impl_mut<T: Allocator<VM>>(
        &mut self,
        selector: AllocatorSelector,
    ) -> &mut T {
        self.allocators.get_typed_allocator_mut(selector)
    }

    /// Return the base offset from a mutator pointer to the allocator specified by the selector.
    pub fn get_allocator_base_offset(selector: AllocatorSelector) -> usize {
        use crate::util::alloc::*;
        use memoffset::offset_of;
        use std::mem::size_of;
        offset_of!(Mutator<VM>, allocators)
            + match selector {
                AllocatorSelector::BumpPointer(index) => {
                    offset_of!(Allocators<VM>, bump_pointer)
                        + size_of::<BumpAllocator<VM>>() * index as usize
                }
                AllocatorSelector::FreeList(index) => {
                    offset_of!(Allocators<VM>, free_list)
                        + size_of::<FreeListAllocator<VM>>() * index as usize
                }
                AllocatorSelector::Immix(index) => {
                    offset_of!(Allocators<VM>, immix)
                        + size_of::<ImmixAllocator<VM>>() * index as usize
                }
                AllocatorSelector::LargeObject(index) => {
                    offset_of!(Allocators<VM>, large_object)
                        + size_of::<LargeObjectAllocator<VM>>() * index as usize
                }
                AllocatorSelector::Malloc(index) => {
                    offset_of!(Allocators<VM>, malloc)
                        + size_of::<MallocAllocator<VM>>() * index as usize
                }
                AllocatorSelector::MarkCompact(index) => {
                    offset_of!(Allocators<VM>, markcompact)
                        + size_of::<MarkCompactAllocator<VM>>() * index as usize
                }
                AllocatorSelector::None => panic!("Expect a valid AllocatorSelector, found None"),
            }
    }
}

/// Each GC plan should provide their implementation of a MutatorContext. *Note that this trait is no longer needed as we removed
/// per-plan mutator implementation and we will remove this trait as well in the future.*

// TODO: We should be able to remove this trait, as we removed per-plan mutator implementation, and there is no other type that implements this trait.
// The Mutator struct above is the only type that implements this trait. We should be able to merge them.
pub trait MutatorContext<VM: VMBinding>: Send + 'static {
    fn prepare(&mut self, tls: VMWorkerThread);
    fn release(&mut self, tls: VMWorkerThread);
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
    ) -> Address;
    fn alloc_slow(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
    ) -> Address;
    fn post_alloc(&mut self, refer: ObjectReference, bytes: usize, allocator: AllocationSemantics);
    fn flush_remembered_sets(&mut self) {
        self.barrier().flush();
    }
    fn flush(&mut self) {
        self.flush_remembered_sets();
    }
    fn get_tls(&self) -> VMMutatorThread;
    /// Get active barrier trait object
    fn barrier(&mut self) -> &mut dyn Barrier<VM>;
    /// Force cast the barrier trait object to a concrete implementation.
    ///
    /// # Safety
    /// The safety of this function is ensured by a down-cast check.
    unsafe fn barrier_impl<B: Barrier<VM>>(&mut self) -> &mut B;
}

/// This is used for plans to indicate the number of allocators reserved for the plan.
/// This is used as a parameter for creating allocator/space mapping.
/// A plan is required to reserve the first few allocators. For example, if n_bump_pointer is 1,
/// it means the first bump pointer allocator will be reserved for the plan (and the plan should
/// initialize its mapping itself), and the spaces in common/base plan will use the following bump
/// pointer allocators.
#[allow(dead_code)]
#[derive(Default)]
pub(crate) struct ReservedAllocators {
    pub n_bump_pointer: u8,
    pub n_large_object: u8,
    pub n_malloc: u8,
    pub n_immix: u8,
    pub n_mark_compact: u8,
    pub n_free_list: u8,
}

impl ReservedAllocators {
    pub const DEFAULT: Self = ReservedAllocators {
        n_bump_pointer: 0,
        n_large_object: 0,
        n_malloc: 0,
        n_immix: 0,
        n_mark_compact: 0,
        n_free_list: 0,
    };
    /// check if the number of each allocator is okay. Panics if any allocator exceeds the max number.
    fn validate(&self) {
        use crate::util::alloc::allocators::*;
        assert!(
            self.n_bump_pointer as usize <= MAX_BUMP_ALLOCATORS,
            "Allocator mapping declared more bump pointer allocators than the max allowed."
        );
        assert!(
            self.n_large_object as usize <= MAX_LARGE_OBJECT_ALLOCATORS,
            "Allocator mapping declared more large object allocators than the max allowed."
        );
        assert!(
            self.n_malloc as usize <= MAX_MALLOC_ALLOCATORS,
            "Allocator mapping declared more malloc allocators than the max allowed."
        );
        assert!(
            self.n_immix as usize <= MAX_IMMIX_ALLOCATORS,
            "Allocator mapping declared more immix allocators than the max allowed."
        );
        assert!(
            self.n_mark_compact as usize <= MAX_MARK_COMPACT_ALLOCATORS,
            "Allocator mapping declared more mark compact allocators than the max allowed."
        );
        assert!(
            self.n_free_list as usize <= MAX_FREE_LIST_ALLOCATORS,
            "Allocator mapping declared more free list allocators than the max allowed."
        );
    }
}

/// Create an allocator mapping for spaces in Common/BasePlan for a plan. A plan should reserve its own allocators.
///
/// # Arguments
/// * `reserved`: the number of reserved allocators for the plan specific policies.
/// * `include_common_plan`: whether the plan uses common plan. If a plan uses CommonPlan, we will initialize allocator mapping for spaces in CommonPlan.
pub(crate) fn create_allocator_mapping(
    mut reserved: ReservedAllocators,
    include_common_plan: bool,
) -> EnumMap<AllocationSemantics, AllocatorSelector> {
    // If we need to add new allocators, or new spaces, we need to make sure the allocator we assign here matches the allocator
    // we used in create_space_mapping(). The easiest way is to add the space/allocator mapping in the same order. So for any modification to this
    // function, please check the other function.

    let mut map = EnumMap::<AllocationSemantics, AllocatorSelector>::default();

    // spaces in base plan

    #[cfg(feature = "code_space")]
    {
        map[AllocationSemantics::Code] = AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;

        map[AllocationSemantics::LargeCode] =
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;
    }

    #[cfg(feature = "ro_space")]
    {
        map[AllocationSemantics::ReadOnly] =
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;
    }

    // spaces in common plan

    if include_common_plan {
        map[AllocationSemantics::Immortal] =
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;

        map[AllocationSemantics::Los] = AllocatorSelector::LargeObject(reserved.n_large_object);
        reserved.n_large_object += 1;

        // TODO: This should be freelist allocator once we use marksweep for nonmoving space.
        map[AllocationSemantics::NonMoving] =
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;
    }

    reserved.validate();
    map
}

/// Create a space mapping for spaces in Common/BasePlan for a plan. A plan should reserve its own allocators.
///
/// # Arguments
/// * `reserved`: the number of reserved allocators for the plan specific policies.
/// * `include_common_plan`: whether the plan uses common plan. If a plan uses CommonPlan, we will initialize allocator mapping for spaces in CommonPlan.
/// * `plan`: the reference to the plan.
pub(crate) fn create_space_mapping<VM: VMBinding>(
    mut reserved: ReservedAllocators,
    include_common_plan: bool,
    plan: &'static dyn Plan<VM = VM>,
) -> Vec<(AllocatorSelector, &'static dyn Space<VM>)> {
    // If we need to add new allocators, or new spaces, we need to make sure the allocator we assign here matches the allocator
    // we used in create_space_mapping(). The easiest way is to add the space/allocator mapping in the same order. So for any modification to this
    // function, please check the other function.

    let mut vec: Vec<(AllocatorSelector, &'static dyn Space<VM>)> = vec![];

    // spaces in BasePlan

    #[cfg(feature = "code_space")]
    {
        vec.push((
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer),
            &plan.base().code_space,
        ));
        reserved.n_bump_pointer += 1;
        vec.push((
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer),
            &plan.base().code_lo_space,
        ));
        reserved.n_bump_pointer += 1;
    }

    #[cfg(feature = "ro_space")]
    {
        vec.push((
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer),
            &plan.base().ro_space,
        ));
        reserved.n_bump_pointer += 1;
    }

    // spaces in CommonPlan

    if include_common_plan {
        vec.push((
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer),
            plan.common().get_immortal(),
        ));
        reserved.n_bump_pointer += 1;
        vec.push((
            AllocatorSelector::LargeObject(reserved.n_large_object),
            plan.common().get_los(),
        ));
        reserved.n_large_object += 1;
        // TODO: This should be freelist allocator once we use marksweep for nonmoving space.
        vec.push((
            AllocatorSelector::BumpPointer(reserved.n_bump_pointer),
            plan.common().get_nonmoving(),
        ));
        reserved.n_bump_pointer += 1;
    }

    reserved.validate();
    vec
}
