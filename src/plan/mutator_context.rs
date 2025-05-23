//! Mutator context for each application thread.

use crate::plan::barriers::Barrier;
use crate::plan::global::Plan;
use crate::plan::AllocationSemantics;
use crate::policy::space::Space;
use crate::util::alloc::allocator::AllocationOptions;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::Allocator;
use crate::util::{Address, ObjectReference};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::MMTK;

use enum_map::EnumMap;

use super::barriers::NoBarrier;

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

/// An mutator prepare implementation for plans that use [`crate::plan::global::CommonPlan`].
#[allow(unused_variables)]
pub(crate) fn common_prepare_func<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Prepare the free list allocator used for non moving
    #[cfg(feature = "marksweep_as_nonmoving")]
    unsafe {
        mutator.allocator_impl_mut_for_semantic::<crate::util::alloc::FreeListAllocator<VM>>(
            AllocationSemantics::NonMoving,
        )
    }
    .prepare();
}

/// A place-holder implementation for `MutatorConfig::release_func` that should not be called.
/// Currently only used by `NoGC`.
pub(crate) fn unreachable_release_func<VM: VMBinding>(
    _mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    unreachable!("`MutatorConfig::release_func` must not be called for the current plan.")
}

/// An mutator release implementation for plans that use [`crate::plan::global::CommonPlan`].
#[allow(unused_variables)]
pub(crate) fn common_release_func<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "marksweep_as_nonmoving")] {
            // Release the free list allocator used for non moving
            unsafe { mutator.allocator_impl_mut_for_semantic::<crate::util::alloc::FreeListAllocator<VM>>(
                AllocationSemantics::NonMoving,
            )}.release();
        } else if #[cfg(feature = "immortal_as_nonmoving")] {
            // Do nothig for the bump pointer allocator
        } else {
            // Reset the Immix allocator
            unsafe { mutator.allocator_impl_mut_for_semantic::<crate::util::alloc::ImmixAllocator<VM>>(
                AllocationSemantics::NonMoving,
            )}.reset();
        }
    }
}

/// A place-holder implementation for `MutatorConfig::release_func` that does nothing.
#[allow(dead_code)]
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

/// Used to build a mutator struct
pub struct MutatorBuilder<VM: VMBinding> {
    barrier: Box<dyn Barrier<VM>>,
    /// The mutator thread that is bound with this Mutator struct.
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
    config: MutatorConfig<VM>,
}

impl<VM: VMBinding> MutatorBuilder<VM> {
    pub fn new(
        mutator_tls: VMMutatorThread,
        mmtk: &'static MMTK<VM>,
        config: MutatorConfig<VM>,
    ) -> Self {
        MutatorBuilder {
            barrier: Box::new(NoBarrier),
            mutator_tls,
            mmtk,
            config,
        }
    }

    pub fn barrier(mut self, barrier: Box<dyn Barrier<VM>>) -> Self {
        self.barrier = barrier;
        self
    }

    pub fn build(self) -> Mutator<VM> {
        Mutator {
            allocators: Allocators::<VM>::new(
                self.mutator_tls,
                self.mmtk,
                &self.config.space_mapping,
            ),
            barrier: self.barrier,
            mutator_tls: self.mutator_tls,
            plan: self.mmtk.get_plan(),
            config: self.config,
        }
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
    pub(crate) allocators: Allocators<VM>,
    /// Holds some thread-local states for the barrier.
    pub barrier: Box<dyn Barrier<VM>>,
    /// The mutator thread that is bound with this Mutator struct.
    pub mutator_tls: VMMutatorThread,
    pub(crate) plan: &'static dyn Plan<VM = VM>,
    pub(crate) config: MutatorConfig<VM>,
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
        let allocator = unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        };
        // The value should be default/unset at the beginning of an allocation request.
        debug_assert!(allocator.get_context().get_alloc_options().is_default());
        allocator.alloc(size, align, offset)
    }

    fn alloc_with_options(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
        options: AllocationOptions,
    ) -> Address {
        let allocator = unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        };
        // The value should be default/unset at the beginning of an allocation request.
        debug_assert!(allocator.get_context().get_alloc_options().is_default());
        allocator.alloc_with_options(size, align, offset, options)
    }

    fn alloc_slow(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
    ) -> Address {
        let allocator = unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        };
        // The value should be default/unset at the beginning of an allocation request.
        debug_assert!(allocator.get_context().get_alloc_options().is_default());
        allocator.alloc_slow(size, align, offset)
    }

    fn alloc_slow_with_options(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
        options: AllocationOptions,
    ) -> Address {
        let allocator = unsafe {
            self.allocators
                .get_allocator_mut(self.config.allocator_mapping[allocator])
        };
        // The value should be default/unset at the beginning of an allocation request.
        debug_assert!(allocator.get_context().get_alloc_options().is_default());
        allocator.alloc_slow_with_options(size, align, offset, options)
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

    /// Get the allocator of a concrete type for the semantic.
    ///
    /// # Safety
    /// The semantic needs to match the allocator type.
    pub unsafe fn allocator_impl_for_semantic<T: Allocator<VM>>(
        &self,
        semantic: AllocationSemantics,
    ) -> &T {
        self.allocator_impl::<T>(self.config.allocator_mapping[semantic])
    }

    /// Get the mutable allocator of a concrete type for the semantic.
    ///
    /// # Safety
    /// The semantic needs to match the allocator type.
    pub unsafe fn allocator_impl_mut_for_semantic<T: Allocator<VM>>(
        &mut self,
        semantic: AllocationSemantics,
    ) -> &mut T {
        self.allocator_impl_mut::<T>(self.config.allocator_mapping[semantic])
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
    /// Do the prepare work for this mutator.
    fn prepare(&mut self, tls: VMWorkerThread);
    /// Do the release work for this mutator.
    fn release(&mut self, tls: VMWorkerThread);
    /// Allocate memory for an object. This function will trigger a GC on failed allocation.
    ///
    /// Arguments:
    /// * `size`: the number of bytes required for the object.
    /// * `align`: required alignment for the object.
    /// * `offset`: offset associated with the alignment. The result plus the offset will be aligned to the given alignment.
    /// * `allocator`: the allocation semantic used for this object.
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
    ) -> Address;
    /// Allocate memory for an object with more options to control this allocation request, e.g. not triggering a GC on fail.
    ///
    /// Arguments:
    /// * `size`: the number of bytes required for the object.
    /// * `align`: required alignment for the object.
    /// * `offset`: offset associated with the alignment. The result plus the offset will be aligned to the given alignment.
    /// * `allocator`: the allocation semantic used for this object.
    /// * `options`: the allocation options to change the default allocation behavior for this request.
    fn alloc_with_options(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
        options: AllocationOptions,
    ) -> Address;
    /// The slow path allocation for [`MutatorContext::alloc`]. This function will trigger a GC on failed allocation.
    ///
    ///  This is only useful when the binding
    /// implements the fast path allocation, and would like to explicitly
    /// call the slow path after the fast path allocation fails.
    fn alloc_slow(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
    ) -> Address;
    /// The slow path allocation for [`MutatorContext::alloc_with_options`].
    ///
    /// This is only useful when the binding
    /// implements the fast path allocation, and would like to explicitly
    /// call the slow path after the fast path allocation fails.
    fn alloc_slow_with_options(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        allocator: AllocationSemantics,
        options: AllocationOptions,
    ) -> Address;
    /// Perform post-allocation actions.  For many allocators none are
    /// required.
    ///
    /// Arguments:
    /// * `refer`: the newly allocated object.
    /// * `bytes`: the size of the space allocated (in bytes).
    /// * `allocator`: the allocation semantic used.
    fn post_alloc(&mut self, refer: ObjectReference, bytes: usize, allocator: AllocationSemantics);
    /// Flush per-mutator remembered sets and create GC work for the remembered sets.
    fn flush_remembered_sets(&mut self) {
        self.barrier().flush();
    }
    /// Flush the mutator context.
    fn flush(&mut self) {
        self.flush_remembered_sets();
    }
    /// Get the mutator thread for this mutator context. This is the same value as the argument supplied in
    /// [`crate::memory_manager::bind_mutator`] when this mutator is created.
    fn get_tls(&self) -> VMMutatorThread;
    /// Get active barrier trait object
    fn barrier(&mut self) -> &mut dyn Barrier<VM>;
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

    // We may add more allocators from common/base plan after reserved allocators.

    fn add_bump_pointer_allocator(&mut self) -> AllocatorSelector {
        let selector = AllocatorSelector::BumpPointer(self.n_bump_pointer);
        self.n_bump_pointer += 1;
        selector
    }
    fn add_large_object_allocator(&mut self) -> AllocatorSelector {
        let selector = AllocatorSelector::LargeObject(self.n_large_object);
        self.n_large_object += 1;
        selector
    }
    #[allow(dead_code)]
    fn add_malloc_allocator(&mut self) -> AllocatorSelector {
        let selector = AllocatorSelector::Malloc(self.n_malloc);
        self.n_malloc += 1;
        selector
    }
    #[allow(dead_code)]
    fn add_immix_allocator(&mut self) -> AllocatorSelector {
        let selector = AllocatorSelector::Immix(self.n_immix);
        self.n_immix += 1;
        selector
    }
    #[allow(dead_code)]
    fn add_mark_compact_allocator(&mut self) -> AllocatorSelector {
        let selector = AllocatorSelector::MarkCompact(self.n_mark_compact);
        self.n_mark_compact += 1;
        selector
    }
    #[allow(dead_code)]
    fn add_free_list_allocator(&mut self) -> AllocatorSelector {
        let selector = AllocatorSelector::FreeList(self.n_free_list);
        self.n_free_list += 1;
        selector
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
        map[AllocationSemantics::Code] = reserved.add_bump_pointer_allocator();
        map[AllocationSemantics::LargeCode] = reserved.add_bump_pointer_allocator();
    }

    #[cfg(feature = "ro_space")]
    {
        map[AllocationSemantics::ReadOnly] = reserved.add_bump_pointer_allocator();
    }

    // spaces in common plan

    if include_common_plan {
        map[AllocationSemantics::Immortal] = reserved.add_bump_pointer_allocator();
        map[AllocationSemantics::Los] = reserved.add_large_object_allocator();
        map[AllocationSemantics::NonMoving] = if cfg!(feature = "marksweep_as_nonmoving") {
            reserved.add_free_list_allocator()
        } else if cfg!(feature = "immortal_as_nonmoving") {
            reserved.add_bump_pointer_allocator()
        } else {
            reserved.add_immix_allocator()
        };
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
            reserved.add_bump_pointer_allocator(),
            &plan.base().code_space,
        ));
        vec.push((
            reserved.add_bump_pointer_allocator(),
            &plan.base().code_lo_space,
        ));
    }

    #[cfg(feature = "ro_space")]
    vec.push((reserved.add_bump_pointer_allocator(), &plan.base().ro_space));

    // spaces in CommonPlan

    if include_common_plan {
        vec.push((
            reserved.add_bump_pointer_allocator(),
            plan.common().get_immortal(),
        ));
        vec.push((
            reserved.add_large_object_allocator(),
            plan.common().get_los(),
        ));
        vec.push((
            if cfg!(feature = "marksweep_as_nonmoving") {
                reserved.add_free_list_allocator()
            } else if cfg!(feature = "immortal_as_nonmoving") {
                reserved.add_bump_pointer_allocator()
            } else {
                reserved.add_immix_allocator()
            },
            plan.common().get_nonmoving(),
        ));
    }

    reserved.validate();
    vec
}
