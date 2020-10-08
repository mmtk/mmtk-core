use std::mem::MaybeUninit;
use downcast_rs::Downcast;

use crate::vm::VMBinding;
use crate::policy::space::Space;
use crate::util::alloc::{Allocator, BumpAllocator, LargeObjectAllocator};
use crate::util::OpaquePointer;
use crate::plan::selected_plan::SelectedPlan;
use crate::policy::largeobjectspace::LargeObjectSpace;

const MAX_BUMP_ALLOCATORS: usize = 5;
const MAX_LARGE_OBJECT_ALLOCATORS: usize = 1;

// This struct is part of Mutator. 
// We are trying to make it fixed-sized so that VM bindings can easily define a Mutator type to have the exact same layout as our Mutator struct.
pub struct Allocators<VM: VMBinding> {
    pub bump_pointer: [MaybeUninit<BumpAllocator<VM>>; MAX_BUMP_ALLOCATORS],
    pub large_object: [MaybeUninit<LargeObjectAllocator<VM>>; MAX_LARGE_OBJECT_ALLOCATORS],
}

impl<VM: VMBinding> Allocators<VM> {
    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_allocator(&self, selector: AllocatorSelector) -> &dyn Allocator<VM> {
        match selector {
            AllocatorSelector::BumpPointer(index) => self.bump_pointer[index as usize].get_ref(),
            AllocatorSelector::LargeObject(index) => self.large_object[index as usize].get_ref(),
        }
    }

    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_allocator_mut(&mut self, selector: AllocatorSelector) -> &mut dyn Allocator<VM> {
        match selector {
            AllocatorSelector::BumpPointer(index) => self.bump_pointer[index as usize].get_mut(),
            AllocatorSelector::LargeObject(index) => self.large_object[index as usize].get_mut(),
        }        
    }

    pub fn new(mutator_tls: OpaquePointer, plan: &'static SelectedPlan<VM>, space_mapping: &Vec<(AllocatorSelector, &'static dyn Space<VM>)>) -> Self {
        let mut ret = Allocators {
            bump_pointer: unsafe { MaybeUninit::uninit().assume_init() },
            large_object: unsafe { MaybeUninit::uninit().assume_init() },
        };

        for &(selector, space) in space_mapping.iter() {
            match selector {
                AllocatorSelector::BumpPointer(index) => { 
                    ret.bump_pointer[index as usize].write(BumpAllocator::new(mutator_tls, Some(space), plan));
                }
                AllocatorSelector::LargeObject(index) => {
                    ret.large_object[index as usize].write(LargeObjectAllocator::new(mutator_tls, Some(space.downcast_ref::<LargeObjectSpace<VM>>().unwrap()), plan));
                }
            }
        }

        ret
    }
}

// This type is equivalent to:
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
    LargeObject(u8)
}
