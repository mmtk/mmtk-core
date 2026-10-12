use std::mem::{offset_of, MaybeUninit};
use std::sync::Arc;

use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::marksweepspace::malloc_ms::MallocSpace;
use crate::policy::marksweepspace::native_ms::MarkSweepSpace;
use crate::policy::space::Space;
use crate::policy::spanspace::SpanSpace;
use crate::util::alloc::LargeObjectAllocator;
use crate::util::alloc::MallocAllocator;
use crate::util::alloc::{Allocator, BumpAllocator, ImmixAllocator};
use crate::util::VMMutatorThread;
use crate::vm::VMBinding;
use crate::Mutator;
use crate::MMTK;

use super::allocator::AllocatorContext;
use super::FreeListAllocator;
use super::Lisp2Allocator;
use super::SpanAllocator;

pub(crate) const MAX_BUMP_ALLOCATORS: usize = 6;
pub(crate) const MAX_LARGE_OBJECT_ALLOCATORS: usize = 2;
pub(crate) const MAX_MALLOC_ALLOCATORS: usize = 1;
pub(crate) const MAX_IMMIX_ALLOCATORS: usize = 2;
pub(crate) const MAX_FREE_LIST_ALLOCATORS: usize = 2;
pub(crate) const MAX_LISP2_ALLOCATORS: usize = 1;
pub(crate) const MAX_SPAN_ALLOCATORS: usize = 2;

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
    pub lisp2: [MaybeUninit<Lisp2Allocator<VM>>; MAX_LISP2_ALLOCATORS],
    pub span: [MaybeUninit<SpanAllocator<VM>>; MAX_SPAN_ALLOCATORS],
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
            AllocatorSelector::Lisp2(index) => self.lisp2[index as usize].assume_init_ref(),
            AllocatorSelector::Span(index) => self.span[index as usize].assume_init_ref(),
            AllocatorSelector::None => panic!("Allocator mapping is not initialized"),
        }
    }

    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_typed_allocator<T: Allocator<VM>>(&self, selector: AllocatorSelector) -> &T {
        self.get_allocator(selector).downcast_ref().unwrap()
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
            AllocatorSelector::Lisp2(index) => self.lisp2[index as usize].assume_init_mut(),
            AllocatorSelector::Span(index) => self.span[index as usize].assume_init_mut(),
            AllocatorSelector::None => panic!("Allocator mapping is not initialized"),
        }
    }

    /// # Safety
    /// The selector needs to be valid, and points to an allocator that has been initialized.
    pub unsafe fn get_typed_allocator_mut<T: Allocator<VM>>(
        &mut self,
        selector: AllocatorSelector,
    ) -> &mut T {
        self.get_allocator_mut(selector).downcast_mut().unwrap()
    }

    pub fn new(
        mutator_tls: VMMutatorThread,
        mmtk: &MMTK<VM>,
        space_mapping: &[(AllocatorSelector, &'static dyn Space<VM>)],
    ) -> Self {
        let mut ret = Allocators {
            bump_pointer: unsafe { MaybeUninit::uninit().assume_init() },
            large_object: unsafe { MaybeUninit::uninit().assume_init() },
            malloc: unsafe { MaybeUninit::uninit().assume_init() },
            immix: unsafe { MaybeUninit::uninit().assume_init() },
            free_list: unsafe { MaybeUninit::uninit().assume_init() },
            lisp2: unsafe { MaybeUninit::uninit().assume_init() },
            span: unsafe { MaybeUninit::uninit().assume_init() },
        };
        let context = Arc::new(AllocatorContext::new(mmtk));

        for &(selector, space) in space_mapping.iter() {
            match selector {
                AllocatorSelector::BumpPointer(index) => {
                    ret.bump_pointer[index as usize].write(BumpAllocator::new(
                        mutator_tls.0,
                        space,
                        context.clone(),
                    ));
                }
                AllocatorSelector::LargeObject(index) => {
                    ret.large_object[index as usize].write(LargeObjectAllocator::new(
                        mutator_tls.0,
                        space.downcast_ref::<LargeObjectSpace<VM>>().unwrap(),
                        context.clone(),
                    ));
                }
                AllocatorSelector::Malloc(index) => {
                    ret.malloc[index as usize].write(MallocAllocator::new(
                        mutator_tls.0,
                        space.downcast_ref::<MallocSpace<VM>>().unwrap(),
                        context.clone(),
                    ));
                }
                AllocatorSelector::Immix(index) => {
                    ret.immix[index as usize].write(ImmixAllocator::new(
                        mutator_tls.0,
                        Some(space),
                        context.clone(),
                        false,
                    ));
                }
                AllocatorSelector::FreeList(index) => {
                    ret.free_list[index as usize].write(FreeListAllocator::new(
                        mutator_tls.0,
                        space.downcast_ref::<MarkSweepSpace<VM>>().unwrap(),
                        context.clone(),
                    ));
                }
                AllocatorSelector::Lisp2(index) => {
                    ret.lisp2[index as usize].write(Lisp2Allocator::new(
                        mutator_tls.0,
                        space,
                        context.clone(),
                    ));
                }
                AllocatorSelector::Span(index) => {
                    ret.span[index as usize].write(SpanAllocator::new(
                        mutator_tls.0,
                        space.downcast_ref::<SpanSpace<VM>>().unwrap(),
                        context.clone(),
                    ));
                }
                AllocatorSelector::None => panic!("Allocator mapping is not initialized"),
            }
        }

        ret
    }
}

/// This type describe an allocator in the [`crate::Mutator`].
/// For some VM bindings, they may need to access this type from native code. This type is equivalent to the following native types:
/// #[repr(C)]
/// struct AllocatorSelector {
///   tag: AllocatorSelectorTag,
///   payload: u8,
/// }
/// #[repr(u8)]
/// enum AllocatorSelectorTag {
///   BumpPointer,
///   LargeObject,
///   ...
/// }
#[repr(C, u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum AllocatorSelector {
    /// Represents a [`crate::util::alloc::bumpallocator::BumpAllocator`].
    BumpPointer(u8),
    /// Represents a [`crate::util::alloc::large_object_allocator::LargeObjectAllocator`].
    LargeObject(u8),
    /// Represents a [`crate::util::alloc::malloc_allocator::MallocAllocator`].
    Malloc(u8),
    /// Represents a [`crate::util::alloc::immix_allocator::ImmixAllocator`].
    Immix(u8),
    /// Represents a [`crate::util::alloc::lisp2_allocator::Lisp2Allocator`].
    Lisp2(u8),
    /// Represents a [`crate::util::alloc::free_list_allocator::FreeListAllocator`].
    FreeList(u8),
    /// Represents a [`crate::util::alloc::span_allocator::SpanAllocator`].
    Span(u8),
    /// No allocator found.
    #[default]
    None,
}

/// This type describes allocator information. It is used to
/// generate fast paths for the GC. All offset fields are relative to [`Mutator`].
#[repr(C, u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum AllocatorInfo {
    /// This allocator uses a [`crate::util::alloc::bumpallocator::BumpPointer`] as its fastpath.
    BumpPointer {
        /// The byte offset from the mutator's pointer to the [`crate::util::alloc::bumpallocator::BumpPointer`].
        bump_pointer_offset: usize,
    },
    /// This allocator is a [`crate::util::alloc::SpanAllocator`] (a span-based free-cell
    /// allocator, Go's `mcache`). A fast path looks up the allocator's cached span for the span
    /// class of the allocation (`size_class << 1 | noscan`, see
    /// [`crate::policy::spanspace::classes`]), and allocates the object at index
    /// `freeindex + ctz(alloc_cache)` if `alloc_cache != 0`, the index is below `nelems`, and the
    /// next index is not a multiple of 64 (Go's `nextFreeFast`). It then updates `alloc_cache`,
    /// `freeindex` and `alloc_count`. If the span's `needzero` is set, the object must be zeroed.
    /// Otherwise it calls the slow path.
    Span {
        /// The byte offset from the mutator's pointer to the allocator's array of span pointers,
        /// indexed by span class.
        spans_offset: usize,
        /// Offsets of the fields in a span descriptor.
        alloc_cache_offset: usize,
        freeindex_offset: usize,
        nelems_offset: usize,
        elemsize_offset: usize,
        start_offset: usize,
        alloc_count_offset: usize,
        needzero_offset: usize,
    },
    /// This allocator uses a fastpath, but we haven't implemented it yet.
    // FIXME: Add free-list fast-path
    Unimplemented,
    /// This allocator does not have a fastpath.
    #[default]
    None,
}

impl AllocatorInfo {
    /// Return an AllocatorInfo for the given allocator selector. This method is provided
    /// so that VM compilers may generate allocator fast-path and load fields for the fast-path.
    ///
    /// Arguments:
    /// * `selector`: The allocator selector to query.
    pub fn new<VM: VMBinding>(selector: AllocatorSelector) -> AllocatorInfo {
        let base_offset = Mutator::<VM>::get_allocator_base_offset(selector);
        match selector {
            AllocatorSelector::BumpPointer(_) => {
                let bump_pointer_offset = offset_of!(BumpAllocator<VM>, bump_pointer);

                AllocatorInfo::BumpPointer {
                    bump_pointer_offset: base_offset + bump_pointer_offset,
                }
            }

            AllocatorSelector::Immix(_) => {
                let bump_pointer_offset = offset_of!(ImmixAllocator<VM>, bump_pointer);

                AllocatorInfo::BumpPointer {
                    bump_pointer_offset: base_offset + bump_pointer_offset,
                }
            }

            AllocatorSelector::Lisp2(_) => {
                let bump_offset = base_offset + offset_of!(Lisp2Allocator<VM>, bump_allocator);
                let bump_pointer_offset = offset_of!(BumpAllocator<VM>, bump_pointer);

                AllocatorInfo::BumpPointer {
                    bump_pointer_offset: bump_offset + bump_pointer_offset,
                }
            }

            AllocatorSelector::FreeList(_) => AllocatorInfo::Unimplemented,

            AllocatorSelector::Span(_) => {
                use crate::policy::spanspace::Span;
                AllocatorInfo::Span {
                    spans_offset: base_offset + offset_of!(SpanAllocator<VM>, alloc),
                    alloc_cache_offset: offset_of!(Span, alloc_cache),
                    freeindex_offset: offset_of!(Span, freeindex),
                    nelems_offset: offset_of!(Span, nelems),
                    elemsize_offset: offset_of!(Span, elemsize),
                    start_offset: offset_of!(Span, start),
                    alloc_count_offset: offset_of!(Span, alloc_count),
                    needzero_offset: offset_of!(Span, needzero),
                }
            }
            _ => AllocatorInfo::None,
        }
    }
}
