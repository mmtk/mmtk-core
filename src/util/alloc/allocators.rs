use std::mem::MaybeUninit;

use crate::plan::Plan;
use crate::plan::AllocationSemantics;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::util::alloc::LargeObjectAllocator;
use crate::util::alloc::MallocAllocator;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::VMMutatorThread;
use crate::vm::VMBinding;

use enum_map::EnumMap;

const MAX_BUMP_ALLOCATORS: usize = 5;
const MAX_LARGE_OBJECT_ALLOCATORS: usize = 2;
const MAX_MALLOC_ALLOCATORS: usize = 1;

// The allocators set owned by each mutator. We provide a fixed number of allocators for each allocator type in the mutator,
// and each plan will select part of the allocators to use.
// Note that this struct is part of the Mutator struct.
// We are trying to make it fixed-sized so that VM bindings can easily define a Mutator type to have the exact same layout as our Mutator struct.
#[repr(C)]
pub struct Allocators<VM: VMBinding> {
    pub bump_pointer: [MaybeUninit<BumpAllocator<VM>>; MAX_BUMP_ALLOCATORS],
    pub large_object: [MaybeUninit<LargeObjectAllocator<VM>>; MAX_LARGE_OBJECT_ALLOCATORS],
    pub malloc: [MaybeUninit<MallocAllocator<VM>>; MAX_MALLOC_ALLOCATORS],
}

impl<VM: VMBinding> Allocators<VM> {
    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_allocator(&self, selector: AllocatorSelector) -> &dyn Allocator<VM> {
        match selector {
            AllocatorSelector::BumpPointer(index) => {
                self.bump_pointer[index as usize].assume_init_ref()
            }
            AllocatorSelector::LargeObject(index) => {
                self.large_object[index as usize].assume_init_ref()
            }
            AllocatorSelector::Malloc(index) => self.malloc[index as usize].assume_init_ref(),
            AllocatorSelector::None => panic!("Allocator mapping is not initialized"),
        }
    }

    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_allocator_mut(
        &mut self,
        selector: AllocatorSelector,
    ) -> &mut dyn Allocator<VM> {
        match selector {
            AllocatorSelector::BumpPointer(index) => {
                self.bump_pointer[index as usize].assume_init_mut()
            }
            AllocatorSelector::LargeObject(index) => {
                self.large_object[index as usize].assume_init_mut()
            }
            AllocatorSelector::Malloc(index) => self.malloc[index as usize].assume_init_mut(),
            AllocatorSelector::None => panic!("Allocator mapping is not initialized"),
        }
    }

    pub fn new(
        mutator_tls: VMMutatorThread,
        plan: &'static dyn Plan<VM = VM>,
        space_mapping: &[(AllocatorSelector, &'static dyn Space<VM>)],
    ) -> Self {
        let mut ret = Allocators {
            bump_pointer: unsafe { MaybeUninit::uninit().assume_init() },
            large_object: unsafe { MaybeUninit::uninit().assume_init() },
            malloc: unsafe { MaybeUninit::uninit().assume_init() },
        };

        for &(selector, space) in space_mapping.iter() {
            match selector {
                AllocatorSelector::BumpPointer(index) => {
                    ret.bump_pointer[index as usize].write(BumpAllocator::new(
                        mutator_tls.0,
                        space,
                        plan,
                    ));
                }
                AllocatorSelector::LargeObject(index) => {
                    ret.large_object[index as usize].write(LargeObjectAllocator::new(
                        mutator_tls.0,
                        space.downcast_ref::<LargeObjectSpace<VM>>().unwrap(),
                        plan,
                    ));
                }
                AllocatorSelector::Malloc(index) => {
                    ret.malloc[index as usize].write(MallocAllocator::new(
                        mutator_tls.0,
                        space.downcast_ref::<MallocSpace<VM>>().unwrap(),
                        plan,
                    ));
                }
                AllocatorSelector::None => panic!("Allocator mapping is not initialized"),
            }
        }

        ret
    }
}

// This type describe which allocator in the allocators set.
// For VM binding implementors, this type is equivalent to the following native types:
// #[repr(C)]
// struct AllocatorSelector {
//   tag: AllocatorSelectorTag,
//   payload: u8,
// }
// #[repr(u8)]
// enum AllocatorSelectorTag {
//   BumpPointer,
//   LargeObject,
// }
#[repr(C, u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AllocatorSelector {
    BumpPointer(u8),
    LargeObject(u8),
    Malloc(u8),
    None,
}

impl Default for AllocatorSelector {
    fn default() -> Self {
        AllocatorSelector::None
    }
}

/// This is used for plans to indicate the number of allocators reserved for the plan.
/// This is used as a parameter to base_allocator_mapping() or common_allocator_mapping().
#[derive(Default)]
pub(crate) struct ReservedAllocators {
    pub n_bump_pointer: u8,
    pub n_large_object: u8,
    pub n_malloc: u8,
}

impl ReservedAllocators {
    /// check if the number of each allocators is okay. Panics if any allocator exceeds the max number.
    fn validate(&self) {
        assert!(self.n_bump_pointer as usize <= MAX_BUMP_ALLOCATORS, "Allocator mapping declared more bump pointer allocators than the max allowed.");
        assert!(self.n_large_object as usize<= MAX_LARGE_OBJECT_ALLOCATORS, "Allocator mapping declared more large object allocators than the max allowed.");
        assert!(self.n_malloc as usize <= MAX_MALLOC_ALLOCATORS, "Allocator mapping declared more malloc allocators than the max allowed.");
    }
}

/// Create a default allocator mapping for BasePlan spaces. Only plans that use BasePlan instead of CommonPlan should use this (e.g. NoGC).
/// Other plans should use common_allocator_mapping().
///
/// # Arguments
/// * `reserved`: the number of reserved allocators for the plan specific policies.
#[allow(unused_mut)] // allow unused mut as some spaces are conditionally compiled
pub(crate) fn base_allocator_mapping(mut reserved: ReservedAllocators) -> EnumMap<AllocationSemantics, AllocatorSelector> {
    let mut base = EnumMap::<AllocationSemantics, AllocatorSelector>::default();

    #[cfg(feature = "code_space")]
    {
        assert_eq!(base[AllocationSemantics::Code], AllocatorSelector::None);
        base[AllocationSemantics::Code] = AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;

        assert_eq!(base[AllocationSemantics::LargeCode], AllocatorSelector::None);
        base[AllocationSemantics::LargeCode] = AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;
    }

    #[cfg(feature = "ro_space")]
    {
        assert_eq!(base[AllocationSemantics::ReadOnly], AllocatorSelector::None);
        base[AllocationSemantics::ReadOnly] = AllocatorSelector::BumpPointer(reserved.n_bump_pointer);
        reserved.n_bump_pointer += 1;
    }

    reserved.validate();
    base
}

/// Create a default allocator mapping for CommonPlan spaces. Most plans that use CommonPlan should use this to create allocator mapping.
///
/// # Arguments
/// * `reserved`: the number of reserved allocators for the plan specific policies.
pub(crate) fn common_allocator_mapping(reserved: ReservedAllocators) -> EnumMap<AllocationSemantics, AllocatorSelector> {
    let mut common = base_allocator_mapping(ReservedAllocators {
        n_bump_pointer: reserved.n_bump_pointer + 1,
        n_large_object: reserved.n_large_object + 1,
        n_malloc: reserved.n_malloc
    });

    assert_eq!(common[AllocationSemantics::Immortal], AllocatorSelector::None);
    common[AllocationSemantics::Immortal] = AllocatorSelector::BumpPointer(reserved.n_bump_pointer);

    assert_eq!(common[AllocationSemantics::Los], AllocatorSelector::None);
    common[AllocationSemantics::Los] = AllocatorSelector::LargeObject(reserved.n_large_object);

    common
}

#[allow(unused_mut)] // allow unused mut as some spaces are conditionally compiled
#[allow(unused_variables)]
pub(crate) fn base_space_mapping<VM: VMBinding>(mut reserved: ReservedAllocators, plan: &'static dyn Plan<VM=VM>) -> Vec<(AllocatorSelector, &'static dyn Space<VM>)> {
    let mut base = vec![];

    #[cfg(feature = "code_space")]
    {
        base.push((AllocatorSelector::BumpPointer(reserved.n_bump_pointer), plan.base().code_space));
        reserved.n_bump_pointer += 1;
        base.push((AllocatorSelector::BumpPointer(reserved.n_bump_pointer), plan.base().code_lo_space));
        reserved.n_bump_pointer += 1;
    }

    #[cfg(feature = "ro_space")]
    {
        base.push((AllocatorSelector::BumpPointer(reserved.n_bump_pointer), plan.base().ro_space));
        reserved.n_bump_pointer += 1;
    }

    reserved.validate();
    base
}

pub(crate) fn common_space_mapping<VM: VMBinding>(reserved: ReservedAllocators, plan: &'static dyn Plan<VM=VM>) -> Vec<(AllocatorSelector, &'static dyn Space<VM>)> {
    let mut common = base_space_mapping(ReservedAllocators {
        n_bump_pointer: reserved.n_bump_pointer + 1,
        n_large_object: reserved.n_large_object + 1,
        n_malloc: reserved.n_malloc
    }, plan);

    common.push((AllocatorSelector::BumpPointer(reserved.n_bump_pointer), plan.common().get_immortal()));
    common.push((AllocatorSelector::LargeObject(reserved.n_large_object), plan.common().get_los()));

    common
}
