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

pub struct Allocators<VM: VMBinding> {
    pub bump_pointer: [MaybeUninit<BumpAllocator<VM>>; MAX_BUMP_ALLOCATORS],
    pub large_object: [MaybeUninit<LargeObjectAllocator<VM>>; MAX_LARGE_OBJECT_ALLOCATORS],
}

impl<VM: VMBinding> Allocators<VM> {
    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_allocator(&self, selector: AllocatorSelector) -> &dyn Allocator<VM> {
        match selector {
            AllocatorSelector::BumpPointer(index) => self.bump_pointer[index].get_ref(),
            AllocatorSelector::LargeObject(index) => self.large_object[index].get_ref(),
        }
    }

    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_allocator_mut(&mut self, selector: AllocatorSelector) -> &mut dyn Allocator<VM> {
        match selector {
            AllocatorSelector::BumpPointer(index) => self.bump_pointer[index].get_mut(),
            AllocatorSelector::LargeObject(index) => self.large_object[index].get_mut(),
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
                    ret.bump_pointer[index].write(BumpAllocator::new(mutator_tls, Some(space), plan));
                }
                AllocatorSelector::LargeObject(index) => {
                    ret.large_object[index].write(LargeObjectAllocator::new(mutator_tls, Some(space.downcast_ref::<LargeObjectSpace<VM>>().unwrap()), plan));
                }
            }
        }

        ret
    }
}

#[derive(Copy, Clone)]
pub enum AllocatorSelector {
    BumpPointer(usize),
    LargeObject(usize)
}