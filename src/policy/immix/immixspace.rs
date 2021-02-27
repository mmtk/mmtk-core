use crate::{plan::TransitiveClosure, util::{OpaquePointer, constants::{LOG_BYTES_IN_PAGE, LOG_BYTES_IN_WORD}, heap::FreeListPageResource}};
use crate::plan::{AllocationSemantics, CopyContext};
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::constants::CARD_META_PAGES_PER_REGION;
use crate::util::forwarding_word as ForwardingWord;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::conversions;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::util::side_metadata::{self, *};
use libc::{mprotect, PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::{cell::UnsafeCell, collections::HashSet, sync::Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

unsafe impl<VM: VMBinding> Sync for ImmixSpace<VM> {}

const LOG_PAGES_IN_BLOCK: usize = 3;
const PAGES_IN_BLOCK: usize = 1 << LOG_PAGES_IN_BLOCK;
const LOG_BYTES_IN_BLOCK: usize = LOG_PAGES_IN_BLOCK + LOG_BYTES_IN_PAGE as usize;
const BYTES_IN_BLOCK: usize = 1 << LOG_BYTES_IN_BLOCK;

const META_BLOCK_MARK: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: 0,
    log_num_of_bits: 0,
    log_min_obj_size: LOG_BYTES_IN_BLOCK,
};

const META_OBJECT_MARK: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: META_BLOCK_MARK.meta_bytes_per_chunk(),
    log_num_of_bits: 0,
    log_min_obj_size: LOG_BYTES_IN_WORD as usize,
};

// const GLOBAL_META_2: SideMetadataSpec = SideMetadataSpec {
//    scope: SideMetadataScope::Global,
//    offset: meta_bytes_per_chunk(s1, b1),
//    log_num_of_bits: b2,
//    log_min_obj_size: s2,
// };

pub struct ImmixSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: FreeListPageResource<VM>,
    all_regions: Mutex<HashSet<Address>>,
}

impl<VM: VMBinding> SFT for ImmixSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        ForwardingWord::is_forwarded::<VM>(object)
    }
    fn is_movable(&self) -> bool {
        true
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        !self.from_space()
    }
    fn initialize_header(&self, _object: ObjectReference, _alloc: bool) {}
}

impl<VM: VMBinding> Space<VM> for ImmixSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }
    fn common(&self) -> &CommonSpace<VM> {
        unsafe { &*self.common.get() }
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        &mut *self.common.get()
    }
    fn init(&mut self, _vm_map: &'static VMMap) {
        println!("Init Space {:?}", self as *const _);
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };
        self.pr.bind_space(me);
        self.common().init(self.as_space());
    }
    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immixspace only releases pages enmasse")
    }

    fn local_side_metadata_per_chunk(&self) -> usize {
        META_BLOCK_MARK.meta_bytes_per_chunk() + META_OBJECT_MARK.meta_bytes_per_chunk()
    }
}

impl<VM: VMBinding> ImmixSpace<VM> {
    pub fn new(
        name: &'static str,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                zeroed: true,
                vmrequest: VMRequest::discontiguous(),
            },
            vm_map,
            mmapper,
            heap,
        );
        ImmixSpace {
            pr: if common.vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common: UnsafeCell::new(common),
            all_regions: Default::default(),
        }
    }

    pub fn defrag_headroom_pages(&self) -> usize {
        self.pr.reserved_pages() * 2 / 100
    }

    pub fn prepare(&self) {
        let mut all_regions = self.all_regions.lock().unwrap();
        for region in all_regions.iter() {
            unsafe { side_metadata::store_atomic(META_BLOCK_MARK, *region, 0); }
            unsafe { side_metadata::bzero_metadata_for_chunk(META_OBJECT_MARK, conversions::chunk_align_down(*region)); }
        }
    }

    pub fn release(&self) {
        let mut all_regions = self.all_regions.lock().unwrap();
        let unmarked_regions = all_regions.drain_filter(|r| {
            unsafe { side_metadata::load(META_BLOCK_MARK, *r) == 0 }
        });
        for region in unmarked_regions {
            self.pr.release_pages(region);
        }
    }

    pub fn get_space(&self, tls: OpaquePointer) -> Address {
        let region = self.acquire(tls, 8);
        if region.is_zero() { return region }
        let mut all_regions = self.all_regions.lock().unwrap();
        debug_assert!(!all_regions.contains(&region), "Duplicate region {:?}", region);
        all_regions.insert(region);
        // println!("New Region {:?}", region);
        region
    }

    #[inline]
    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, semantics: AllocationSemantics) -> ObjectReference {
        if unsafe { side_metadata::compare_exchange_atomic(META_OBJECT_MARK, object.to_address(), 0, 1) } {
            // Mark block
            let region = object.to_address().align_down(8 * 4096);
            unsafe { side_metadata::compare_exchange_atomic(META_BLOCK_MARK, region, 0, 1); }
            // Visit node
            trace.process_node(object);
        }
        object
    }

    // #[inline]
    // pub fn trace_object<T: TransitiveClosure, C: CopyContext>(
    //     &self,
    //     trace: &mut T,
    //     object: ObjectReference,
    //     semantics: AllocationSemantics,
    //     copy_context: &mut C,
    // ) -> ObjectReference {
    //     trace!("copyspace.trace_object(, {:?}, {:?})", object, semantics,);
    //     if !self.from_space() {
    //         return object;
    //     }
    //     trace!("attempting to forward");
    //     let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
    //     trace!("checking if object is being forwarded");
    //     if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
    //         trace!("... yes it is");
    //         let new_object =
    //             ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
    //         trace!("Returning");
    //         new_object
    //     } else {
    //         trace!("... no it isn't. Copying");
    //         let new_object =
    //             ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context);
    //         trace!("Forwarding pointer");
    //         trace.process_node(new_object);
    //         trace!("Copying [{:?} -> {:?}]", object, new_object);
    //         new_object
    //     }
    // }
}
