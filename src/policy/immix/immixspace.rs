use crate::{plan::TransitiveClosure, util::{OpaquePointer, constants::{LOG_BYTES_IN_WORD}, heap::FreeListPageResource}};
use crate::plan::AllocationSemantics;
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::forwarding_word as ForwardingWord;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::conversions;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::heap::PageResource;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::util::side_metadata::{self, *};
use std::{cell::UnsafeCell, collections::HashSet, sync::Mutex};
use super::block::*;



pub struct ImmixSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: FreeListPageResource<VM>,
    all_regions: Mutex<HashSet<Address>>,
}

unsafe impl<VM: VMBinding> Sync for ImmixSpace<VM> {}

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
        Block::MARK_TABLE.meta_bytes_per_chunk() + Self::OBJECT_MARK_TABLE.meta_bytes_per_chunk()
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
        for region in self.all_regions.lock().unwrap().iter() {
            Block::from(*region).clear_mark();
            side_metadata::bzero_metadata_for_chunk(Self::OBJECT_MARK_TABLE, conversions::chunk_align_down(*region))
        }
    }

    pub fn release(&self) {
        let mut all_regions = self.all_regions.lock().unwrap();
        let unmarked_regions = all_regions.drain_filter(|r| !Block::from(*r).is_marked());
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
    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, _semantics: AllocationSemantics) -> ObjectReference {
        if Self::attempt_mark(object) {
            // Mark block
            Block::containing(object).mark();
            // Visit node
            trace.process_node(object);
        }
        object
    }

    /* Object Marking */

    const OBJECT_MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: Block::MARK_TABLE.meta_bytes_per_chunk(),
        log_num_of_bits: 0,
        log_min_obj_size: LOG_BYTES_IN_WORD as usize,
    };

    #[inline(always)]
    fn attempt_mark(object: ObjectReference) -> bool {
        side_metadata::compare_exchange_atomic(Self::OBJECT_MARK_TABLE, object.to_address(), 0, 1)
    }
}
