use std::mem::size_of;
use std::mem::MaybeUninit;

use memoffset::offset_of;

use crate::plan::Plan;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::marksweepspace::malloc_ms::MallocSpace;
use crate::policy::marksweepspace::native_ms::MarkSweepSpace;
use crate::policy::space::Space;
use crate::util::alloc::LargeObjectAllocator;
use crate::util::alloc::MallocAllocator;
use crate::util::alloc::{Allocator, BumpAllocator, ImmixAllocator};
use crate::util::VMMutatorThread;
use crate::vm::VMBinding;
use crate::Mutator;

use super::FreeListAllocator;
use super::MarkCompactAllocator;

pub(crate) const MAX_BUMP_ALLOCATORS: usize = 6;
pub(crate) const MAX_LARGE_OBJECT_ALLOCATORS: usize = 2;
pub(crate) const MAX_MALLOC_ALLOCATORS: usize = 1;
pub(crate) const MAX_IMMIX_ALLOCATORS: usize = 1;
pub(crate) const MAX_FREE_LIST_ALLOCATORS: usize = 2;
pub(crate) const MAX_MARK_COMPACT_ALLOCATORS: usize = 1;

// The allocators set owned by each mutator. We provide a fixed number of allocators for each allocator type in the mutator,
// and each plan will select part of the allocators to use.
// Note that this struct is part of the Mutator struct.
// We are trying to make it fixed-sized so that VM bindings can easily define a Mutator type to have the exact same layout as our Mutator struct.
#[repr(C)]
pub struct Allocators<VM: VMBinding> {
    pub bump_pointer: [MaybeUninit<BumpAllocator<VM>>; MAX_BUMP_ALLOCATORS],
    pub large_object: [MaybeUninit<LargeObjectAllocator<VM>>; MAX_LARGE_OBJECT_ALLOCATORS],
    pub malloc: [MaybeUninit<MallocAllocator<VM>>; MAX_MALLOC_ALLOCATORS],
    pub immix: [MaybeUninit<ImmixAllocator<VM>>; MAX_IMMIX_ALLOCATORS],
    pub free_list: [MaybeUninit<FreeListAllocator<VM>>; MAX_FREE_LIST_ALLOCATORS],
    pub markcompact: [MaybeUninit<MarkCompactAllocator<VM>>; MAX_MARK_COMPACT_ALLOCATORS],
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
            AllocatorSelector::Immix(index) => self.immix[index as usize].assume_init_ref(),
            AllocatorSelector::FreeList(index) => self.free_list[index as usize].assume_init_ref(),
            AllocatorSelector::MarkCompact(index) => {
                self.markcompact[index as usize].assume_init_ref()
            }
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
            AllocatorSelector::Immix(index) => self.immix[index as usize].assume_init_mut(),
            AllocatorSelector::FreeList(index) => self.free_list[index as usize].assume_init_mut(),
            AllocatorSelector::MarkCompact(index) => {
                self.markcompact[index as usize].assume_init_mut()
            }
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
            immix: unsafe { MaybeUninit::uninit().assume_init() },
            free_list: unsafe { MaybeUninit::uninit().assume_init() },
            markcompact: unsafe { MaybeUninit::uninit().assume_init() },
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
                AllocatorSelector::Immix(index) => {
                    ret.immix[index as usize].write(ImmixAllocator::new(
                        mutator_tls.0,
                        Some(space),
                        plan,
                        false,
                    ));
                }
                AllocatorSelector::FreeList(index) => {
                    ret.free_list[index as usize].write(FreeListAllocator::new(
                        mutator_tls.0,
                        space.downcast_ref::<MarkSweepSpace<VM>>().unwrap(),
                        plan,
                    ));
                }
                AllocatorSelector::MarkCompact(index) => {
                    ret.markcompact[index as usize].write(MarkCompactAllocator::new(
                        mutator_tls.0,
                        space,
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AllocatorSelector {
    BumpPointer(u8),
    LargeObject(u8),
    Malloc(u8),
    Immix(u8),
    MarkCompact(u8),
    FreeList(u8),
    None,
}

impl Default for AllocatorSelector {
    fn default() -> Self {
        AllocatorSelector::None
    }
}

/// This type describes allocator information. It is used to
/// generate fast paths for the GC. All offset fields are relative to [Mutator](crate::Mutator).
#[repr(C, u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AllocatorInfo {
    BumpPointer {
        limit_offset: usize,
        cursor_offset: usize,
    },
    // FIXME: Add free-list fast-path
    Unimplemented,
    None,
}

impl Default for AllocatorInfo {
    fn default() -> Self {
        AllocatorInfo::None
    }
}

impl AllocatorInfo {
    /// Return an AllocatorInfo for the given allocator selector. This method is provided
    /// so that VM compilers may generate allocator fast-path and load fields for the fast-path.
    ///
    /// Arguments:
    /// * `selector`: The allocator selector to query.
    pub fn new<VM: VMBinding>(selector: AllocatorSelector) -> AllocatorInfo {
        match selector {
            AllocatorSelector::BumpPointer(index) => {
                let base_offset = offset_of!(Mutator<VM>, allocators)
                    + offset_of!(Allocators<VM>, bump_pointer)
                    + size_of::<BumpAllocator<VM>>() * index as usize;
                let limit_offset = base_offset + offset_of!(BumpAllocator<VM>, limit);
                let cursor_offset = base_offset + offset_of!(BumpAllocator<VM>, cursor);

                AllocatorInfo::BumpPointer {
                    limit_offset,
                    cursor_offset,
                }
            }

            AllocatorSelector::Immix(index) => {
                let base_offset = offset_of!(Mutator<VM>, allocators)
                    + offset_of!(Allocators<VM>, immix)
                    + size_of::<ImmixAllocator<VM>>() * index as usize;
                let limit_offset = base_offset + offset_of!(ImmixAllocator<VM>, limit);
                let cursor_offset = base_offset + offset_of!(ImmixAllocator<VM>, cursor);
                AllocatorInfo::BumpPointer {
                    limit_offset,
                    cursor_offset,
                }
            }

            AllocatorSelector::MarkCompact(index) => {
                let base_offset = offset_of!(Mutator<VM>, allocators)
                    + offset_of!(Allocators<VM>, markcompact)
                    + size_of::<MarkCompactAllocator<VM>>() * index as usize;
                let bump_offset =
                    base_offset + offset_of!(MarkCompactAllocator<VM>, bump_allocator);
                let limit_offset = bump_offset + offset_of!(BumpAllocator<VM>, limit);
                let cursor_offset = bump_offset + offset_of!(BumpAllocator<VM>, cursor);
                AllocatorInfo::BumpPointer {
                    limit_offset,
                    cursor_offset,
                }
            }

            AllocatorSelector::FreeList(_) => AllocatorInfo::Unimplemented,
            _ => AllocatorInfo::None,
        }
    }
}
