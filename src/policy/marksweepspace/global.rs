use std::{collections::{HashMap, HashSet}, sync::Mutex};

use atomic::Ordering;

use crate::{TransitiveClosure, policy::marksweepspace::metadata::{ALLOC_SIDE_METADATA_SPEC, is_marked, set_mark_bit, unset_mark_bit}, util::{Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread, alloc::free_list_allocator::{self, BYTES_IN_BLOCK, FreeListAllocator}, heap::{FreeListPageResource, HeapMeta, VMRequest, layout::heap_layout::{Mmapper, VMMap}}, metadata::{self, MetadataSpec, compare_exchange_metadata, load_metadata, side_metadata::{LOCAL_SIDE_METADATA_BASE_ADDRESS, SideMetadataContext, SideMetadataSpec, metadata_address_range_size}, store_metadata}}, vm::VMBinding};

use super::{super::space::{CommonSpace, SFT, Space, SpaceOptions}, metadata::{is_alloced, unset_alloc_bit}};
use crate::vm::ObjectModel;

pub struct MarkSweepSpace<VM: VMBinding> {
    pub active_blocks: Mutex<HashSet<Address>>,
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    marked_blocks: HashMap<usize, Vec<free_list_allocator::BlockQueue>>
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
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        // local_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> MarkSweepSpace<VM> {
        let alloc_mark_bits = &mut metadata::extract_side_metadata(&[
            MetadataSpec::OnSide(ALLOC_SIDE_METADATA_SPEC),
            VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        ]);
        let side_metadata_next = SideMetadataSpec {
            is_global: false,
            offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_size = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_local_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_thread_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_tls = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free) + metadata_address_range_size(&side_metadata_thread_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };

        let side_metadata_marked = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free) + metadata_address_range_size(&side_metadata_thread_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let mut local_specs = {
            vec![
                side_metadata_next,
                side_metadata_free,
                side_metadata_size,
                side_metadata_local_free,
                side_metadata_thread_free,
                side_metadata_tls,
                side_metadata_marked,
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
        if !is_marked::<VM>(object) {
            set_mark_bit::<VM>(object);
            let block = FreeListAllocator::<VM>::get_block(address);
            self.mark_block(block);
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

    #[inline]
    pub fn get_marked_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[6]
    }

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
            let marked = is_marked::<VM>(unsafe { cell.to_object_reference() });
            if alloced {
                if !marked {
                    self.free(cell, tls);
                }
                else {
                    unset_mark_bit::<VM>(unsafe{cell.to_object_reference()});
                }
            }
            cell += cell_size;
        }
    }

    pub fn reset(&mut self) {
        // zero marked blocks
        self.marked_blocks = HashMap::default();
    }

    pub fn block_level_sweep(&mut self) {
        let mut block = self.common.start;
        // safe to assume start is block aligned?
        while block < self.common.start + self.common.extent {
            // eprintln!("block level sweep: looking at block {}", &block);
            if self.alloced_block(block) {
                if self.marked_block(block) {
                    let tls = self.load_block_tls(block);
                    let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(tls) };
                    self.marked_blocks.insert(tls, free_list_allocator::BLOCK_QUEUES_EMPTY.to_vec());
                } else {
                    self.block_clear_metadata(block);
                    self.return_block(block);
                }
            }
            block +=  BYTES_IN_BLOCK;
        }
        eprintln!("Done with block level sweep!");
    }

    pub fn return_block(&self, block: Address) {
        // block entirely freed
        // todo!

        // not sure how to do this, so for the moment I will ignore it - this space will never be allocated to again
    }

    pub fn marked_block(&self, block: Address) -> bool {
        load_metadata::<VM>(
            MetadataSpec::OnSide(self.get_marked_metadata_spec()), 
            unsafe {block.to_object_reference()},
            None,
            Some(Ordering::SeqCst)) == 1
    }

    pub fn mark_block(&self, block: Address) {
        store_metadata::<VM>(
            MetadataSpec::OnSide(self.get_marked_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            1, 
            None, 
            None
        );
    }

    pub fn alloced_block(&self, block: Address) -> bool {
        load_metadata::<VM>(
            MetadataSpec::OnSide(self.get_tls_metadata_spec()), 
            unsafe {block.to_object_reference()},
            None,
            Some(Ordering::SeqCst)) != 0
    }

    pub fn block_clear_metadata(&self, block: Address) {
        for metadata_spec in &self.common.metadata.local {
            store_metadata::<VM>(
                MetadataSpec::OnSide(*metadata_spec),
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
            MetadataSpec::OnSide(self.get_tls_metadata_spec()), 
            unsafe {block.to_object_reference()},
            None,
            Some(Ordering::SeqCst));
        unsafe {
            std::mem::transmute::<usize, OpaquePointer>(tls)
        }
    }

    pub fn load_block_cell_size(&self, block: Address) -> usize {
        load_metadata::<VM>(
            MetadataSpec::OnSide(self.get_size_metadata_spec()), 
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
                        MetadataSpec::OnSide(self.get_local_free_metadata_spec()), 
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
                MetadataSpec::OnSide(self.get_free_metadata_spec()),
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
                            MetadataSpec::OnSide(self.get_thread_free_metadata_spec()), 
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
                    MetadataSpec::OnSide(self.get_thread_free_metadata_spec()),
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
        unset_alloc_bit(unsafe { addr.to_object_reference() });

    }
}