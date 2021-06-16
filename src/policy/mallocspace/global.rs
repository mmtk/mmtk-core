use super::metadata::*;
use crate::plan::TransitiveClosure;
use crate::policy::space::CommonSpace;
use crate::policy::space::SFT;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::conversions;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::PageResource;
use crate::util::malloc::*;
use crate::util::opaque_pointer::*;
use crate::util::side_metadata::SideMetadataSanity;
use crate::util::side_metadata::{SideMetadata, SideMetadataContext, SideMetadataSpec};
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
// use crate::util::side_metadata::bzero_metadata;
use crate::vm::{ActivePlan, Collection, ObjectModel};
use crate::{policy::space::Space, util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::marker::PhantomData;
// only used for debugging
#[cfg(debug_assertions)]
use std::collections::HashMap;
#[cfg(debug_assertions)]
use std::sync::Mutex;

// If true, we will use a hashmap to store all the allocated memory from malloc, and use it
// to make sure our allocation is correct.
#[cfg(debug_assertions)]
const ASSERT_ALLOCATION: bool = false;

pub struct MallocSpace<VM: VMBinding> {
    phantom: PhantomData<VM>,
    active_bytes: AtomicUsize,
    pub chunk_addr_min: AtomicUsize, // XXX: have to use AtomicUsize to represent an Address
    pub chunk_addr_max: AtomicUsize,
    metadata: SideMetadata,
    // Mapping between allocated address and its size - this is used to check correctness.
    // Size will be set to zero when the memory is freed.
    #[cfg(debug_assertions)]
    active_mem: Mutex<HashMap<Address, usize>>,
    #[cfg(debug_assertions)]
    pub total_work_packets: AtomicUsize,
    #[cfg(debug_assertions)]
    pub completed_work_packets: AtomicUsize,
    #[cfg(debug_assertions)]
    pub work_live_bytes: AtomicUsize,
}

impl<VM: VMBinding> SFT for MallocSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        is_marked(object)
    }

    fn is_movable(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        trace!("initialize_header for object {}", object);
        let page_addr = conversions::page_align_down(object.to_address()); // XXX: page-bit diff
        set_page_mark_bit(page_addr); // XXX: page-bit diff
        set_alloc_bit(object);
    }
}

impl<VM: VMBinding> Space<VM> for MallocSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }

    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        unreachable!()
    }

    fn common(&self) -> &CommonSpace<VM> {
        unreachable!()
    }

    fn init(&mut self, _vm_map: &'static VMMap) {
        // Do nothing
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        unreachable!()
    }

    // We have assertions in a debug build. We allow this pattern for the release build.
    #[allow(clippy::let_and_return)]
    fn in_space(&self, object: ObjectReference) -> bool {
        let ret = is_alloced_by_malloc(object);

        #[cfg(debug_assertions)]
        if ASSERT_ALLOCATION {
            let addr = VM::VMObjectModel::object_start_ref(object);
            let active_mem = self.active_mem.lock().unwrap();
            if ret {
                // The alloc bit tells that the object is in space.
                debug_assert!(
                    *active_mem.get(&addr).unwrap() != 0,
                    "active mem check failed for {} (object {}) - was freed",
                    addr,
                    object
                );
            } else {
                // The alloc bit tells that the object is not in space. It could never be allocated, or have been freed.
                debug_assert!(
                    (!active_mem.contains_key(&addr))
                        || (active_mem.contains_key(&addr) && *active_mem.get(&addr).unwrap() == 0),
                    "mem check failed for {} (object {}): allocated = {}, size = {:?}",
                    addr,
                    object,
                    active_mem.contains_key(&addr),
                    if active_mem.contains_key(&addr) {
                        active_mem.get(&addr)
                    } else {
                        None
                    }
                );
            }
        }
        ret
    }

    fn address_in_space(&self, _start: Address) -> bool {
        unreachable!("We do not know if an address is in malloc space. Use in_space() to check if an object is in malloc space.")
    }

    fn get_name(&self) -> &'static str {
        "MallocSpace"
    }

    fn reserved_pages(&self) -> usize {
        let data_pages = conversions::bytes_to_pages_up(self.active_bytes.load(Ordering::SeqCst));
        let meta_pages = self.metadata.calculate_reserved_pages(data_pages);
        data_pages + meta_pages
    }

    fn verify_side_metadata_sanity(&self, side_metadata_sanity_checker: &mut SideMetadataSanity) {
        side_metadata_sanity_checker
            .verify_metadata_context(std::any::type_name::<Self>(), self.metadata.get_context())
    }
}

impl<VM: VMBinding> MallocSpace<VM> {
    pub fn new(global_side_metadata_specs: Vec<SideMetadataSpec>) -> Self {
        MallocSpace {
            phantom: PhantomData,
            active_bytes: AtomicUsize::new(0),
            chunk_addr_min: AtomicUsize::new(usize::max_value()), // XXX: have to use AtomicUsize to represent an Address
            chunk_addr_max: AtomicUsize::new(0),
            metadata: SideMetadata::new(SideMetadataContext {
                global: global_side_metadata_specs,
                local: vec![ALLOC_METADATA_SPEC, MARKING_METADATA_SPEC, ACTIVE_PAGE_METADATA_SPEC],
            }),
            #[cfg(debug_assertions)]
            active_mem: Mutex::new(HashMap::new()),
            #[cfg(debug_assertions)]
            total_work_packets: AtomicUsize::new(0),
            #[cfg(debug_assertions)]
            completed_work_packets: AtomicUsize::new(0),
            #[cfg(debug_assertions)]
            work_live_bytes: AtomicUsize::new(0),
        }
    }

    pub fn alloc(&self, tls: VMThread, size: usize) -> Address {
        // TODO: Should refactor this and Space.acquire()
        if VM::VMActivePlan::global().poll(false, self) {
            assert!(VM::VMActivePlan::is_mutator(tls), "Polling in GC worker");
            VM::VMCollection::block_for_gc(VMMutatorThread(tls));
            return unsafe { Address::zero() };
        }

        let raw = unsafe { calloc(1, size) };
        let address = Address::from_mut_ptr(raw);

        // if calloc returns a valid pointer
        if !address.is_zero() {
            let actual_size = unsafe { malloc_usable_size(raw) };
            // Check if the metadata space for the associated chunk has been mapped
            if !is_meta_space_mapped(address) {
                let chunk_start = conversions::chunk_align_down(address);
                debug!(
                    "Add malloc chunk {} to {}",
                    chunk_start,
                    chunk_start + BYTES_IN_CHUNK
                );
                // Map the metadata space for the associated chunk
                self.map_meta_space_for_chunk(chunk_start);
            }
            self.active_bytes.fetch_add(actual_size, Ordering::SeqCst);

            #[cfg(debug_assertions)]
            if ASSERT_ALLOCATION {
                debug_assert!(actual_size != 0);
                self.active_mem.lock().unwrap().insert(address, actual_size);
            }
        }

        address
    }

    pub fn free(&self, addr: Address) {
        let ptr = addr.to_mut_ptr();
        let bytes = unsafe { malloc_usable_size(ptr) };
        trace!("Free memory {:?}", ptr);
        unsafe {
            free(ptr);
        }
        self.active_bytes.fetch_sub(bytes, Ordering::SeqCst);

        #[cfg(debug_assertions)]
        if ASSERT_ALLOCATION {
            self.active_mem.lock().unwrap().insert(addr, 0).unwrap();
        }
    }

    #[inline]
    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let address = object.to_address();
        // let page_addr = conversions::page_align_down(address);
        let chunk_start = conversions::chunk_align_down(address);
        assert!(
            self.in_space(object),
            "Cannot mark an object {} that was not alloced by malloc.",
            address,
        );

        if !is_marked(object) {
            set_mark_bit(object);
            // set_page_mark_bit(page_addr);
            set_chunk_mark_bit(chunk_start);
            trace.process_node(object);
        }

        object
    }

    #[inline]
    fn map_meta_space_for_chunk(&self, chunk_start: Address) {
        // Map the metadata space for chunk
        map_chunk_meta_space(&self.metadata, chunk_start);

        // Update the bounds of the max and min chunk addresses seen -- this is used later in the sweep
        // Lockless compare-and-swap loops perform better than a locking variant
        let chunk_usize = chunk_start.as_usize();
        let mut min = self.chunk_addr_min.load(Ordering::Relaxed);
        let mut max = self.chunk_addr_max.load(Ordering::Relaxed);

        while chunk_usize < min {
            match self.chunk_addr_min.compare_exchange_weak(
                min,
                chunk_usize,
                Ordering::AcqRel,
                Ordering::Relaxed
                ) {
                Ok(_) => break,
                Err(x) => min = x,
            }
        }

        while chunk_usize > max {
            match self.chunk_addr_max.compare_exchange_weak(
                max,
                chunk_usize,
                Ordering::AcqRel,
                Ordering::Relaxed
                ) {
                Ok(_) => break,
                Err(x) => max = x,
            }
        }
    }

    /// # Safety
    /// unsafe as it uses non-atomic accesses to side metadata (although these
    /// non-atomic accesses should not have race conditions associated with them)
    /// as well as calling libc functions
    pub unsafe fn sweep_chunk(&self, chunk_start: Address) {
        #[cfg(debug_assertions)]
        let mut live_bytes = 0;

        debug!("Check active chunk {:?}", chunk_start);
        let mut chunk_is_empty = true;
        let mut address = chunk_start;
        let chunk_end = chunk_start + BYTES_IN_CHUNK;
        let mut page = conversions::page_align_down(address); // XXX: page-bit diff
        let mut page_is_empty = true; // XXX: page-bit diff
        let mut on_page_boundary = false;

        // Linear scan through the chunk
        while address < chunk_end {
            trace!("Check address {}", address);

            if address - page >= BYTES_IN_PAGE { // XXX: page-bit diff
                if page_is_empty {
                    unset_page_mark_bit_unsafe(page);
                }
                page = conversions::page_align_down(address);
                page_is_empty = !on_page_boundary;
                on_page_boundary = false;
            }

            // Check if the address is an object
            if is_alloced_object_unsafe(address) {
                let object = address.to_object_reference();
                let obj_start = VM::VMObjectModel::object_start_ref(object);
                let bytes = malloc_usable_size(obj_start.to_mut_ptr());

                #[cfg(debug_assertions)]
                if ASSERT_ALLOCATION {
                    debug_assert!(
                        self.active_mem.lock().unwrap().contains_key(&obj_start),
                        "Address {} with alloc bit is not in active_mem",
                        obj_start
                    );
                    debug_assert_eq!(
                        self.active_mem.lock().unwrap().get(&obj_start),
                        Some(&bytes),
                        "Address {} size in active_mem does not match the size from malloc_usable_size",
                        obj_start
                    );
                }

                if !is_marked_unsafe(object) {
                    // Dead object
                    trace!(
                        "Object {} has alloc bit but no mark bit, it is dead. ",
                        object
                    );

                    // Free object
                    self.free(obj_start);
                    trace!("free object {}", object);
                    unset_alloc_bit_unsafe(object);
                } else {
                    // Live object. Unset mark bit
                    unset_mark_bit_unsafe(object);
                    // This chunk is still active.
                    chunk_is_empty = false;
                    page_is_empty = false; // XXX: page-bit diff

                    if address + bytes - page > BYTES_IN_PAGE {
                        on_page_boundary = true;
                    }

                    #[cfg(debug_assertions)]
                    {
                        // Accumulate live bytes
                        live_bytes += bytes;
                    }
                }

                // Skip to next object
                address += bytes;
            } else { // not an object
                address += VM::MIN_ALIGNMENT;
            }
        }

        if chunk_is_empty {
            unset_chunk_mark_bit_unsafe(chunk_start);
        }

        debug!(
            "Used bytes after releasing: {}",
            self.active_bytes.load(Ordering::SeqCst)
        );

        #[cfg(debug_assertions)]
        {
            let completed_packets = self.completed_work_packets.fetch_add(1, Ordering::SeqCst) + 1;
            self.work_live_bytes.fetch_add(live_bytes, Ordering::SeqCst);

            if completed_packets == self.total_work_packets.load(Ordering::Relaxed) {
                info!(
                    "work_live_bytes = {}, live_bytes = {}, active_bytes = {}",
                    self.work_live_bytes.load(Ordering::Relaxed),
                    live_bytes,
                    self.active_bytes.load(Ordering::Relaxed)
                );
                debug_assert_eq!(
                    self.work_live_bytes.load(Ordering::Relaxed),
                    self.active_bytes.load(Ordering::Relaxed)
                );
            }
        }
    }
}
