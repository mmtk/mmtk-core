use std::mem::MaybeUninit;

use crate::vm::VMBinding;
use crate::policy::space::Space;
use crate::util::alloc::{Allocator, BumpAllocator, LargeObjectAllocator};

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

    pub fn uninit() -> Self {
        let uninit = Allocators {
            bump_pointer: unsafe { MaybeUninit::uninit().assume_init() },
            large_object: unsafe { MaybeUninit::uninit().assume_init() },
        };

        uninit
    }
}

#[derive(Copy, Clone)]
pub enum AllocatorSelector {
    BumpPointer(usize),
    LargeObject(usize)
}