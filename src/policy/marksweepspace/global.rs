use std::{collections::{HashMap, HashSet}, ops::DerefMut, sync::{Arc, Mutex}};

use atomic::Ordering;

use crate::{TransitiveClosure, policy::marksweepspace::{block::{Block, BlockState}, metadata::{ALLOC_SIDE_METADATA_SPEC, is_marked, set_mark_bit, unset_mark_bit}}, scheduler::{MMTkScheduler, WorkBucketStage}, util::{Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread, alloc::free_list_allocator::{self, BYTES_IN_BLOCK, FreeListAllocator}, heap::{FreeListPageResource, HeapMeta, VMRequest, layout::heap_layout::{Mmapper, VMMap}}, metadata::{self, MetadataSpec, compare_exchange_metadata, load_metadata, side_metadata::{LOCAL_SIDE_METADATA_BASE_ADDRESS, SideMetadataContext, SideMetadataOffset, SideMetadataSpec}, store_metadata}}, vm::VMBinding};

use super::{super::space::{CommonSpace, SFT, Space, SpaceOptions}, chunks::ChunkMap, metadata::{is_alloced, unset_alloc_bit_unsafe}};
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
    pub active_blocks: Mutex<HashSet<Address>>,
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    pub marked_blocks: HashMap<usize, Vec<free_list_allocator::BlockQueue>>,
    /// Allocation status for all chunks in immix space
    pub chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<MMTkScheduler<VM>>,
}

impl<VM: VMBinding> SFT for MarkSweepSpace<VM> {
    fn name(&self) -> &str {
        self.common.name
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
        // do nothing
    }
}

impl<VM: VMBinding> Space<VM> for MarkSweepSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }

    fn get_page_resource(&self) -> &dyn crate::util::heap::PageResource<VM> {
        &self.pr
    }

    fn init(&mut self, vm_map: &'static crate::util::heap::layout::heap_layout::VMMap) {
        self.common().init(self.as_space());
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, start: crate::util::Address) {
        todo!()
    }
}

impl<VM: VMBinding> MarkSweepSpace<VM> {
    /// Get work packet scheduler
    fn scheduler(&self) -> &MMTkScheduler<VM> {
        &self.scheduler
    }

    pub fn new(
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
        let side_metadata_next = SideMetadataSpec {
            is_global: false,
            offset: SideMetadataOffset::layout_after(&ChunkMap::ALLOC_TABLE),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_free = SideMetadataSpec {
            is_global: false,
            offset: SideMetadataOffset::layout_after(&side_metadata_next),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_size = SideMetadataSpec {
            is_global: false,
            offset: SideMetadataOffset::layout_after(&side_metadata_free),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_local_free = SideMetadataSpec {
            is_global: false,
            offset: SideMetadataOffset::layout_after(&side_metadata_size),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_thread_free = SideMetadataSpec {
            is_global: false,
            offset: SideMetadataOffset::layout_after(&side_metadata_local_free),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_tls = SideMetadataSpec {
            is_global: false,
            offset: SideMetadataOffset::layout_after(&side_metadata_thread_free),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };

        // let side_metadata_marked = SideMetadataSpec {
        //     is_global: false,
        //     offset: SideMetadataOffset::layout_after(&side_metadata_tls),
        //     log_num_of_bits: 6,
        //     log_min_obj_size: 16,
        // };
        let mut local_specs = {
            vec![
                side_metadata_next,
                side_metadata_free,
                side_metadata_size,
                side_metadata_local_free,
                side_metadata_thread_free,
                side_metadata_tls,
                // side_metadata_marked,
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
                    local: local_specs
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        MarkSweepSpace {
            active_blocks: Mutex::default(),
            pr: if vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common,
            marked_blocks: HashMap::default(),
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
        if !is_marked::<VM>(object, None) {
            set_mark_bit::<VM>(object, Some(Ordering::SeqCst));
            let block = Block::from(FreeListAllocator::<VM>::get_block(address));
            block.set_state(BlockState::Marked);
            // self.mark_block(block);
            trace.process_node(object);
        }
        object
    }

    #[inline]
    pub fn get_next_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[0]
    }

    #[inline]
    pub fn get_free_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[1]
    }

    #[inline]
    pub fn get_size_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[2]
    }

    #[inline]
    pub fn get_local_free_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[3]
    }

    #[inline]
    pub fn get_thread_free_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[4]
    }

    #[inline]
    pub fn get_tls_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[5]
    }

    // #[inline]
    // pub fn get_marked_metadata_spec(&self) -> SideMetadataSpec {
    //     self.common.metadata.local[6]
    // }

    pub fn eager_sweep(&self, tls: VMWorkerThread) {
        let active_blocks = &*self.active_blocks.lock().unwrap();
        for block in active_blocks {
            self.sweep_block(*block, tls)
        }
    }

    pub fn sweep_block(&self, block: Address, tls: VMWorkerThread) {
        // eprintln!("Sweep block {}", block);
        let cell_size = self.load_block_cell_size(block);
        let mut cell = block;
        while cell < block + BYTES_IN_BLOCK {
            // eprintln!("look at cell {}", cell);
            let alloced = is_alloced(unsafe { cell.to_object_reference() });
            let marked = is_marked::<VM>(unsafe { cell.to_object_reference() }, Some(Ordering::SeqCst));
            if alloced {
                if !marked {
                    self.free(cell, tls);
                }
                else {
                    unset_mark_bit::<VM>(unsafe{cell.to_object_reference()}, Some(Ordering::SeqCst));
                }
            }
            cell += cell_size;
        }
    }

    pub fn reset(&mut self) {
        // zero marked blocks
        self.marked_blocks = HashMap::default();
    }

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
            &MetadataSpec::OnSide(self.get_tls_metadata_spec()), 
            unsafe {block.to_object_reference()},
            None,
            Some(Ordering::SeqCst)) != 0
    }

    pub fn block_clear_metadata(&self, block: Address) {
        for metadata_spec in &self.common.metadata.local {
            store_metadata::<VM>(
                &MetadataSpec::OnSide(*metadata_spec),
                unsafe{block.to_object_reference()},
                0,
                None,
                Some(Ordering::SeqCst)
            )
        }
    }
    
    pub fn load_block_tls(&self, block: Address) -> OpaquePointer {
        eprintln!("Load tls for block {}", block);
        let tls = load_metadata::<VM>(
            &MetadataSpec::OnSide(self.get_tls_metadata_spec()), 
            unsafe {block.to_object_reference()},
            None,
            Some(Ordering::SeqCst));
        unsafe {
            std::mem::transmute::<usize, OpaquePointer>(tls)
        }
    }

    pub fn load_block_cell_size(&self, block: Address) -> usize {
        load_metadata::<VM>(
            &MetadataSpec::OnSide(self.get_size_metadata_spec()), 
            unsafe {block.to_object_reference()},
            None,
            Some(Ordering::SeqCst))
    }

    pub fn free(&self, addr: Address, tls: VMWorkerThread) {

        let block = FreeListAllocator::<VM>::get_block(addr);
        let block_tls = self.load_block_tls(block);

        if tls.0.0 == block_tls {
            // same thread that allocated
            let local_free = unsafe {
                Address::from_usize(
                    load_metadata::<VM>(
                        &MetadataSpec::OnSide(self.get_local_free_metadata_spec()), 
                        block.to_object_reference(), 
                        None, 
                        None,
                    )
                )
            };
            unsafe {
                addr.store(local_free);
            }
            store_metadata::<VM>(
                &MetadataSpec::OnSide(self.get_free_metadata_spec()),
                unsafe{block.to_object_reference()}, 
                addr.as_usize(), None, 
                None
            );
        } else {
            // different thread to allocator
            let mut success = false;
            while !success {
                let thread_free = unsafe {
                    Address::from_usize(
                        load_metadata::<VM>(
                            &MetadataSpec::OnSide(self.get_thread_free_metadata_spec()), 
                            block.to_object_reference(), 
                            None, 
                            Some(Ordering::SeqCst),
                        )
                    )
                };
                unsafe {
                    addr.store(thread_free);
                }
                success = compare_exchange_metadata::<VM>(
                    &MetadataSpec::OnSide(self.get_thread_free_metadata_spec()),
                    unsafe{block.to_object_reference()}, 
                    thread_free.as_usize(), 
                    addr.as_usize(), 
                    None,
                    Ordering::SeqCst,
                    Ordering::SeqCst, //?
                );
            }
        }
        

        // unset allocation bit
        unsafe { unset_alloc_bit_unsafe(unsafe { addr.to_object_reference() }) };

    }
}

