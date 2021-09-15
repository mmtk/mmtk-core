use crate::{TransitiveClosure, util::{Address, ObjectReference, constants::CARD_META_PAGES_PER_REGION, heap::{FreeListPageResource, HeapMeta, VMRequest, layout::heap_layout::{Mmapper, VMMap}}, side_metadata::{SideMetadataContext, SideMetadataSpec}}, vm::VMBinding};

use crate::{
    policy::marksweepspace::{
        block::{Block, BlockState},
        metadata::{is_marked, set_mark_bit, unset_mark_bit, ALLOC_SIDE_METADATA_SPEC},
    },
    scheduler::{MMTkScheduler, WorkBucketStage},
    util::{
        alloc::free_list_allocator::{self, FreeListAllocator, BLOCK_LISTS_EMPTY, BYTES_IN_BLOCK},
        constants::LOG_BYTES_IN_PAGE,
        heap::{
            layout::heap_layout::{Mmapper, VMMap},
            FreeListPageResource, HeapMeta, VMRequest,
        },
        metadata::{
            self, compare_exchange_metadata, load_metadata,
            side_metadata::{
                SideMetadataContext, SideMetadataOffset, SideMetadataSpec,
                LOCAL_SIDE_METADATA_BASE_ADDRESS,
            },
            store_metadata, MetadataSpec,
        },
        Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread,
    },
    vm::VMBinding,
    TransitiveClosure,
};

use crate::{
    policy::marksweepspace::{
        block::{Block, BlockState},
        metadata::{is_marked, set_mark_bit, unset_mark_bit, ALLOC_SIDE_METADATA_SPEC},
    },
    scheduler::{MMTkScheduler, WorkBucketStage},
    util::{
        alloc::free_list_allocator::{self, FreeListAllocator, BLOCK_LISTS_EMPTY, BYTES_IN_BLOCK},
        constants::LOG_BYTES_IN_PAGE,
        heap::{
            layout::heap_layout::{Mmapper, VMMap},
            FreeListPageResource, HeapMeta, VMRequest,
        },
        metadata::{
            self, compare_exchange_metadata, load_metadata,
            side_metadata::{
                SideMetadataContext, SideMetadataOffset, SideMetadataSpec,
                LOCAL_SIDE_METADATA_BASE_ADDRESS,
            },
            store_metadata, MetadataSpec,
        },
        Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread,
    },
    vm::VMBinding,
    TransitiveClosure,
};

use super::{
    super::space::{CommonSpace, Space, SpaceOptions, SFT},
    chunks::ChunkMap,
    metadata::{is_alloced, unset_alloc_bit_unsafe},
};
use crate::vm::ObjectModel;

// const NATIVE_MALLOC_SPECS: Vec<SideMetadataSpec> = [
//     SideMetadataSpec {
//         is_global: false,
//         offset:
//         log_num_of_bits: 6,
//         log_min_obj_size: 16,
//     },
// ].to_vec();

pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    // pub marked_blocks: Mutex<HashMap<usize, Vec<free_list_allocator::BlockList>>>,
    /// Allocation status for all chunks in immix space
    pub chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<MMTkScheduler<VM>>,
}

impl<VM: VMBinding> SFT for MarkSweepSpace<VM> {
    fn name(&self) -> &str {
        todo!()
    }

    fn is_live(&self, object: crate::util::ObjectReference) -> bool {
        todo!()
    }

    fn is_movable(&self) -> bool {
        todo!()
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        todo!()
    }

    fn initialize_object_metadata(&self, object: crate::util::ObjectReference, alloc: bool) {
        todo!()
    }
}

impl<VM: VMBinding> Space<VM> for MarkSweepSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        todo!()
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        todo!()
    }

    fn get_page_resource(&self) -> &dyn crate::util::heap::PageResource<VM> {
        &self.pr
    }

    fn init(&mut self, vm_map: &'static crate::util::heap::layout::heap_layout::VMMap) {
        todo!()
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, start: crate::util::Address) {
        todo!()
    }
}

impl<VM: VMBinding> MarkSweepSpace<VM> {
    fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        // local_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
        scheduler: Arc<MMTkScheduler<VM>>,
    ) -> MarkSweepSpace<VM> {
        let alloc_mark_bits = &mut metadata::extract_side_metadata(&[
            MetadataSpec::OnSide(ALLOC_SIDE_METADATA_SPEC),
            *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        ]);

        let mut local_specs = {
            vec![
                Block::NEXT_BLOCK_TABLE,
                Block::FREE_LIST_TABLE,
                Block::SIZE_TABLE,
                Block::LOCAL_FREE_LIST_TABLE,
                Block::THREAD_FREE_LIST_TABLE,
                Block::TLS_TABLE,
                Block::MARK_TABLE,
                ChunkMap::ALLOC_TABLE,
            ]
        };

        local_specs.append(alloc_mark_bits);

        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: vec![],
                    local: local_specs,
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        MarkSweepSpace {
            pr: if vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common,
            chunk_map: ChunkMap::new(),
            scheduler,
        }
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let address = object.to_address();
        assert!(
            self.in_space(object),
            "Cannot mark an object {} that was not alloced by free list allocator.",
            address,
        );
        // use crate::util::alloc::free_list_allocator::TRACING_OBJECT;
        // if *TRACING_OBJECT.lock().unwrap() == address.as_usize() {
        //     println!("marking tracing object 0x{:0x}", *TRACING_OBJECT.lock().unwrap());
        // }
        if !is_marked::<VM>(object, None) {
            set_mark_bit::<VM>(object, Some(Ordering::SeqCst));
            let block = Block::from(FreeListAllocator::<VM>::get_block(address));
            block.set_state(BlockState::Marked);
            // self.mark_block(block);
            trace.process_node(object);
        }
        object
    }

    pub fn acquire_block(&self) -> Address {
        // acquire 64kB block from the global pool
        todo!()
    }

    pub fn return_block(&self) {
        // return freed 64kB block
        todo!()
    }

    #[inline]
    pub fn get_next_metadata_spec(&self) -> SideMetadataSpec {
        Block::NEXT_BLOCK_TABLE
    }

    pub fn reset(&mut self) {
        // do nothing
    }

    // pub fn acquire_block_for_size(&self, alloc_tls: VMThread, size: usize) -> AcquireResult {
    //     let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(alloc_tls.0) };
    //     // eprintln!("space: acquire block for size, tls={}", tls);
    //     let block = { let mut marked_blocks = self.marked_blocks.lock().unwrap();
    //         // eprintln!("space: lock on marked blocks, tls={}", tls);
    //         let marked_block_lists = match marked_blocks.get(&tls) {
    //             Some(lists) => lists,
    //             None => {
    //                 marked_blocks.insert(tls, BLOCK_LISTS_EMPTY.to_vec());
    //                 marked_blocks.get(&tls).unwrap()
    //             },
    //         };
    //         let bin = FreeListAllocator::<VM>::mi_bin(size);
    //         let block_queue = marked_block_lists[bin as usize];
    //         block_queue.first
    //     };
    //     let res = if block == unsafe { Address::zero() } {
    //         // no marked blocks, get a fresh one
    //         AcquireResult {
    //             block: self.acquire(alloc_tls, BYTES_IN_BLOCK >> LOG_BYTES_IN_PAGE),
    //             new: true,
    //         }
    //     } else {
    //         // I need to update the marked blocks!! This won't work
    //         AcquireResult {
    //             block,
    //             new: false,
    //         }
    //     };
    //     // eprintln!("space: done acquire block for size, tls={}", tls);
    //     res
    // }

    // pub fn block_level_sweep(&mut self) {
    //     let mut block = self.common.start;
    //     // safe to assume start is block aligned?
    //     while block < self.common.start + self.common.extent {
    //         // eprintln!("block level sweep: looking at block {}", &block);
    //         if self.alloced_block(block) {
    //             if self.marked_block(block) {
    //                 let tls = self.load_block_tls(block);
    //                 let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(tls) };
    //                 self.marked_blocks.insert(tls, free_list_allocator::BLOCK_QUEUES_EMPTY.to_vec());
    //             } else {
    //                 self.block_clear_metadata(block);
    //                 self.return_block(block);
    //             }
    //         }
    //         block +=  BYTES_IN_BLOCK;
    //     }
    //     eprintln!("Done with block level sweep");
    // }

    pub fn block_level_sweep(&self) {
        let space = unsafe { &*(self as *const Self) };
        let work_packets = self.chunk_map.generate_sweep_tasks(space);
        self.scheduler().work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
    }

    /// Release a block.
    pub fn release_block(&self, block: Address) {
        eprintln!("b < 0x{:0x} - 0x{:0x}", block, block + BYTES_IN_BLOCK);
        self.block_clear_metadata(block);
        let block = Block::from(block);
        block.deinit();
        self.pr.release_pages(block.start());
    }

    // pub fn marked_block(&self, block: Address) -> bool {
    //     load_metadata::<VM>(
    //         &MetadataSpec::OnSide(self.get_marked_metadata_spec()),
    //         unsafe {block.to_object_reference()},
    //         None,
    //         Some(Ordering::SeqCst)) == 1
    // }

    // pub fn mark_block(&self, block: Address) {
    //     store_metadata::<VM>(
    //         &MetadataSpec::OnSide(self.get_marked_metadata_spec()),
    //         unsafe{block.to_object_reference()},
    //         1,
    //         None,
    //         None
    //     );
    // }

    pub fn alloced_block(&self, block: Address) -> bool {
        load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::TLS_TABLE),
            unsafe { block.to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        ) != 0
    }

    pub fn block_clear_metadata(&self, block: Address) {
        for metadata_spec in &self.common.metadata.local {
            store_metadata::<VM>(
                &MetadataSpec::OnSide(*metadata_spec),
                unsafe { block.to_object_reference() },
                0,
                None,
                Some(Ordering::SeqCst),
            )
        }
    }

    pub fn load_block_tls(&self, block: Address) -> OpaquePointer {
        let tls = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::TLS_TABLE),
            unsafe { block.to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        );
        unsafe { std::mem::transmute::<usize, OpaquePointer>(tls) }
    }

    pub fn load_block_cell_size(&self, block: Address) -> usize {
        load_metadata::<VM>(
            &MetadataSpec::OnSide(self.get_size_metadata_spec()),
            unsafe { block.to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        )
    }

    //     pub fn free(&self, addr: Address, tls: VMWorkerThread) {

    //         let block = FreeListAllocator::<VM>::get_block(addr);
    //         let block_tls = self.load_block_tls(block);

    //         if tls.0.0 == block_tls {
    //             // same thread that allocated
    //             let local_free = unsafe {
    //                 Address::from_usize(
    //                     load_metadata::<VM>(
    //                         &MetadataSpec::OnSide(self.get_local_free_metadata_spec()),
    //                         block.to_object_reference(),
    //                         None,
    //                         None,
    //                     )
    //                 )
    //             };
    //             unsafe {
    //                 addr.store(local_free);
    //             }
    //             store_metadata::<VM>(
    //                 &MetadataSpec::OnSide(self.get_free_metadata_spec()),
    //                 unsafe{block.to_object_reference()},
    //                 addr.as_usize(), None,
    //                 None
    //             );
    //         } else {
    //             // different thread to allocator
    //             let mut success = false;
    //             while !success {
    //                 let thread_free = unsafe {
    //                     Address::from_usize(
    //                         load_metadata::<VM>(
    //                             &MetadataSpec::OnSide(self.get_thread_free_metadata_spec()),
    //                             block.to_object_reference(),
    //                             None,
    //                             Some(Ordering::SeqCst),
    //                         )
    //                     )
    //                 };
    //                 unsafe {
    //                     addr.store(thread_free);
    //                 }
    //                 success = compare_exchange_metadata::<VM>(
    //                     &MetadataSpec::OnSide(self.get_thread_free_metadata_spec()),
    //                     unsafe{block.to_object_reference()},
    //                     thread_free.as_usize(),
    //                     addr.as_usize(),
    //                     None,
    //                     Ordering::SeqCst,
    //                     Ordering::SeqCst, //?
    //                 );
    //             }
    //         }

    //         // unset allocation bit
    //         unsafe { unset_alloc_bit_unsafe(unsafe { addr.to_object_reference() }) };

    //     }
}

pub struct AcquireResult {
    pub block: Address,
    pub new: bool,
}
