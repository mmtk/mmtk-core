use std::mem::MaybeUninit;

use crate::plan::selected_plan::SelectedPlan;
#[cfg(feature = "largeobjectspace")]
use crate::policy::largeobjectspace::LargeObjectSpace;
#[cfg(feature = "largeobjectspace")]
use crate::util::alloc::LargeObjectAllocator;
use crate::policy::space::Space;
use crate::util::alloc::{Allocator, BumpAllocator, FreeListAllocator};
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

const MAX_BUMP_ALLOCATORS: usize = 5;
const MAX_LARGE_OBJECT_ALLOCATORS: usize = 1;
const MAX_FREE_LIST_ALLOCATORS: usize = 5;

// The allocators set owned by each mutator. We provide a fixed number of allocators for each allocator type in the mutator,
// and each plan will select part of the allocators to use.
// Note that this struct is part of the Mutator struct.
// We are trying to make it fixed-sized so that VM bindings can easily define a Mutator type to have the exact same layout as our Mutator struct.
#[repr(C)]
pub struct Allocators<VM: VMBinding> {
    pub bump_pointer: [MaybeUninit<BumpAllocator<VM>>; MAX_BUMP_ALLOCATORS],
    
    #[cfg(feature = "largeobjectspace")]
    pub large_object: [MaybeUninit<LargeObjectAllocator<VM>>; MAX_LARGE_OBJECT_ALLOCATORS],
    
    pub free_list: [MaybeUninit<FreeListAllocator<VM>>; MAX_FREE_LIST_ALLOCATORS],
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
        }
    }

    pub fn new(
        mutator_tls: OpaquePointer,
        plan: &'static SelectedPlan<VM>,
        space_mapping: &[(AllocatorSelector, &'static dyn Space<VM>)],
    ) -> Self {
        let mut ret = Allocators {
            bump_pointer: unsafe { MaybeUninit::uninit().assume_init() },
            #[cfg(feature = "largeobjectspace")]
            large_object: unsafe { MaybeUninit::uninit().assume_init() },
            free_list: unsafe { MaybeUninit::uninit().assume_init() },
        };

        for &(selector, space) in space_mapping.iter() {
            match selector {
                AllocatorSelector::BumpPointer(index) => {
                    ret.bump_pointer[index as usize].write(BumpAllocator::new(
                        mutator_tls,
                        Some(space),
                        plan,
                    ));
                }
                #[cfg(feature = "largeobjectspace")]
                AllocatorSelector::LargeObject(index) => {
                    ret.large_object[index as usize].write(LargeObjectAllocator::new(
                        mutator_tls,
                        Some(space.downcast_ref::<LargeObjectSpace<VM>>().unwrap()),
                        plan,
                    ));
                }
                AllocatorSelector::FreeList(index) => {
                    ret.free_list[index as usize].write(FreeListAllocator::new(
                        mutator_tls,
                        Some(space),
                        plan,
                    ));
                }
                
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
#[derive(Copy, Clone)]
pub enum AllocatorSelector {
    BumpPointer(u8),
    #[cfg(feature = "largeobjectspace")]
    LargeObject(u8),
    FreeList(u8),
}
