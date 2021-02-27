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
use std::cell::UnsafeCell;
use super::block::*;
use super::line::*;



pub struct ImmixSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: FreeListPageResource<VM>,
    block_list: BlockList,
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
        Self::OBJECT_MARK_TABLE.accumulated_size()
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
            block_list: BlockList::new(),
        }
    }

    pub fn defrag_headroom_pages(&self) -> usize {
        self.pr.reserved_pages() * 2 / 100
    }

    pub fn prepare(&self) {
        for block in self.block_list.iter() {
            block.clear_mark();
            // TODO: clear metadata for a block only
            for line in block.lines() {
                line.clear_mark()
            }
            side_metadata::bzero_metadata_for_chunk(Self::OBJECT_MARK_TABLE, conversions::chunk_align_down(block.start()))
        }
    }

    pub fn release(&self) {
        for block in self.block_list.drain_filter(|block| !block.is_marked()) {
            self.pr.release_pages(block.start());
        }
    }

    pub fn get_space(&self, tls: OpaquePointer) -> Option<Block> {
        let block_address = self.acquire(tls, 8);
        if block_address.is_zero() { return None }
        let block = Block::from(block_address);
        self.block_list.add(block);
        Some(block)
    }

    #[inline]
    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, _semantics: AllocationSemantics) -> ObjectReference {
        if Self::attempt_mark(object) {
            // Mark block
            Block::containing(object).mark();
            Line::containing(object).mark();
            // Visit node
            trace.process_node(object);
        }
        object
    }

    /* Object Marking */

    const OBJECT_MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: Block::MARK_TABLE.accumulated_size(),
        log_num_of_bits: 0,
        log_min_obj_size: LOG_BYTES_IN_WORD as usize,
    };

    #[inline(always)]
    fn attempt_mark(object: ObjectReference) -> bool {
        side_metadata::compare_exchange_atomic(Self::OBJECT_MARK_TABLE, object.to_address(), 0, 1)
    }
}
