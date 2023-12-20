use atomic::Atomic;

use super::metadata::*;
use crate::plan::ObjectQueue;
use crate::plan::VectorObjectQueue;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::CommonSpace;
use crate::scheduler::GCWorkScheduler;
use crate::util::heap::gc_trigger::GCTrigger;
use crate::util::heap::PageResource;
use crate::util::malloc::library::{BYTES_IN_MALLOC_PAGE, LOG_BYTES_IN_MALLOC_PAGE};
use crate::util::malloc::malloc_ms_util::*;
use crate::util::metadata::side_metadata::{
    SideMetadataContext, SideMetadataSanity, SideMetadataSpec,
};
use crate::util::metadata::MetadataSpec;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::{conversions, metadata};
use crate::vm::VMBinding;
use crate::vm::{ActivePlan, Collection, ObjectModel};
use crate::{policy::space::Space, util::heap::layout::vm_layout::BYTES_IN_CHUNK};
#[cfg(debug_assertions)]
use std::collections::HashMap;
use std::marker::PhantomData;
#[cfg(debug_assertions)]
use std::sync::atomic::AtomicU32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::Mutex;
// If true, we will use a hashmap to store all the allocated memory from malloc, and use it
// to make sure our allocation is correct.
#[cfg(debug_assertions)]
const ASSERT_ALLOCATION: bool = false;

/// This space uses malloc to get new memory, and performs mark-sweep for the memory.
pub struct MallocSpace<VM: VMBinding> {
    phantom: PhantomData<VM>,
    active_bytes: AtomicUsize,
    active_pages: AtomicUsize,
    pub chunk_addr_min: Atomic<Address>,
    pub chunk_addr_max: Atomic<Address>,
    metadata: SideMetadataContext,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
    gc_trigger: Arc<GCTrigger<VM>>,
    // Mapping between allocated address and its size - this is used to check correctness.
    // Size will be set to zero when the memory is freed.
    #[cfg(debug_assertions)]
    active_mem: Mutex<HashMap<Address, usize>>,
    // The following fields are used for checking correctness of the parallel sweep implementation
    // as we need to check how many live bytes exist against `active_bytes` when the last sweep
    // work packet is executed
    #[cfg(debug_assertions)]
    pub total_work_packets: AtomicU32,
    #[cfg(debug_assertions)]
    pub completed_work_packets: AtomicU32,
    #[cfg(debug_assertions)]
    pub work_live_bytes: AtomicUsize,
}

impl<VM: VMBinding> SFT for MallocSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        is_marked::<VM>(object, Ordering::SeqCst)
    }

    #[cfg(feature = "object_pinning")]
    fn pin_object(&self, _object: ObjectReference) -> bool {
        false
    }

    #[cfg(feature = "object_pinning")]
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        false
    }

    #[cfg(feature = "object_pinning")]
    fn is_object_pinned(&self, _object: ObjectReference) -> bool {
        false
    }

    fn is_movable(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    // For malloc space, we need to further check the VO bit.
    fn is_in_space(&self, object: ObjectReference) -> bool {
        is_alloced_by_malloc::<VM>(object)
    }

    /// For malloc space, we just use the side metadata.
    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        debug_assert!(!addr.is_zero());
        // `addr` cannot be mapped by us. It should be mapped by the malloc library.
        debug_assert!(!addr.is_mapped());
        has_object_alloced_by_malloc::<VM>(addr).is_some()
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        trace!("initialize_object_metadata for object {}", object);
        set_vo_bit::<VM>(object);
    }

    fn sft_trace_object(
        &self,
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        self.trace_object(queue, object)
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

    fn get_gc_trigger(&self) -> &GCTrigger<VM> {
        self.gc_trigger.as_ref()
    }

    fn initialize_sft(&self, _sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        // Do nothing - we will set sft when we get new results from malloc
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        unreachable!()
    }

    // We have assertions in a debug build. We allow this pattern for the release build.
    #[allow(clippy::let_and_return)]
    fn in_space(&self, object: ObjectReference) -> bool {
        let ret = is_alloced_by_malloc::<VM>(object);

        #[cfg(debug_assertions)]
        if ASSERT_ALLOCATION {
            let addr = object.to_object_start::<VM>();
            let active_mem = self.active_mem.lock().unwrap();
            if ret {
                // The VO bit tells that the object is in space.
                debug_assert!(
                    *active_mem.get(&addr).unwrap() != 0,
                    "active mem check failed for {} (object {}) - was freed",
                    addr,
                    object
                );
            } else {
                // The VO bit tells that the object is not in space. It could never be allocated, or have been freed.
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

    #[allow(clippy::assertions_on_constants)]
    fn reserved_pages(&self) -> usize {
        use crate::util::constants::LOG_BYTES_IN_PAGE;
        // Assume malloc pages are no smaller than 4K pages. Otherwise the substraction below will fail.
        debug_assert!(LOG_BYTES_IN_MALLOC_PAGE >= LOG_BYTES_IN_PAGE);
        let data_pages = self.active_pages.load(Ordering::SeqCst)
            << (LOG_BYTES_IN_MALLOC_PAGE - LOG_BYTES_IN_PAGE);
        let meta_pages = self.metadata.calculate_reserved_pages(data_pages);
        data_pages + meta_pages
    }

    fn verify_side_metadata_sanity(&self, side_metadata_sanity_checker: &mut SideMetadataSanity) {
        side_metadata_sanity_checker
            .verify_metadata_context(std::any::type_name::<Self>(), &self.metadata)
    }
}

use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for MallocSpace<VM> {
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.trace_object(queue, object)
    }

    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        false
    }
}

// Actually no max object size.
#[allow(dead_code)]
pub const MAX_OBJECT_SIZE: usize = crate::util::constants::MAX_INT;

impl<VM: VMBinding> MallocSpace<VM> {
    pub fn extend_global_side_metadata_specs(specs: &mut Vec<SideMetadataSpec>) {
        // MallocSpace needs to use VO bit. If the feature is turned on, the VO bit spec is in the global specs.
        // Otherwise, we manually add it.
        if !cfg!(feature = "vo_bit") {
            specs.push(crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC);
        }
        // MallocSpace also need a global chunk metadata.
        // TODO: I don't know why this is a global spec. Can we replace it with the chunk map (and the local spec used in the chunk map)?
        // One reason could be that the address range in this space is not in our control, and it could be anywhere in the heap, thus we have
        // to make it a global spec. I am not too sure about this.
        specs.push(ACTIVE_CHUNK_METADATA_SPEC);
    }

    pub fn new(args: crate::policy::space::PlanCreateSpaceArgs<VM>) -> Self {
        MallocSpace {
            phantom: PhantomData,
            active_bytes: AtomicUsize::new(0),
            active_pages: AtomicUsize::new(0),
            chunk_addr_min: Atomic::new(Address::MAX),
            chunk_addr_max: Atomic::new(Address::ZERO),
            metadata: SideMetadataContext {
                global: args.global_side_metadata_specs.clone(),
                local: metadata::extract_side_metadata(&[
                    MetadataSpec::OnSide(ACTIVE_PAGE_METADATA_SPEC),
                    MetadataSpec::OnSide(OFFSET_MALLOC_METADATA_SPEC),
                    *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                ]),
            },
            scheduler: args.scheduler.clone(),
            gc_trigger: args.gc_trigger,
            #[cfg(debug_assertions)]
            active_mem: Mutex::new(HashMap::new()),
            #[cfg(debug_assertions)]
            total_work_packets: AtomicU32::new(0),
            #[cfg(debug_assertions)]
            completed_work_packets: AtomicU32::new(0),
            #[cfg(debug_assertions)]
            work_live_bytes: AtomicUsize::new(0),
        }
    }

    /// Set multiple pages, starting from the given address, for the given size, and increase the active page count if we set any page mark in the region.
    /// This is a thread-safe method, and can be used during mutator phase when mutators may access the same page.
    /// Performance-wise, this method may impose overhead, as we are doing a compare-exchange for every page in the range.
    fn set_page_mark(&self, start: Address, size: usize) {
        // Set first page
        let mut page = start.align_down(BYTES_IN_MALLOC_PAGE);
        let mut used_pages = 0;

        // It is important to go to the end of the object, which may span a page boundary
        while page < start + size {
            if compare_exchange_set_page_mark(page) {
                used_pages += 1;
            }

            page += BYTES_IN_MALLOC_PAGE;
        }

        if used_pages != 0 {
            self.active_pages.fetch_add(used_pages, Ordering::SeqCst);
        }
    }

    /// Unset multiple pages, starting from the given address, for the given size, and decrease the active page count if we unset any page mark in the region
    ///
    /// # Safety
    /// We need to ensure that only one GC thread is accessing the range.
    unsafe fn unset_page_mark(&self, start: Address, size: usize) {
        debug_assert!(start.is_aligned_to(BYTES_IN_MALLOC_PAGE));
        debug_assert!(crate::util::conversions::raw_is_aligned(
            size,
            BYTES_IN_MALLOC_PAGE
        ));
        let mut page = start;
        let mut cleared_pages = 0;
        while page < start + size {
            if is_page_marked_unsafe(page) {
                cleared_pages += 1;
                unset_page_mark_unsafe(page);
            }
            page += BYTES_IN_MALLOC_PAGE;
        }

        if cleared_pages != 0 {
            self.active_pages.fetch_sub(cleared_pages, Ordering::SeqCst);
        }
    }

    pub fn alloc(&self, tls: VMThread, size: usize, align: usize, offset: usize) -> Address {
        // TODO: Should refactor this and Space.acquire()
        if self.get_gc_trigger().poll(false, Some(self)) {
            assert!(VM::VMActivePlan::is_mutator(tls), "Polling in GC worker");
            VM::VMCollection::block_for_gc(VMMutatorThread(tls));
            return unsafe { Address::zero() };
        }

        let (address, is_offset_malloc) = alloc::<VM>(size, align, offset);
        if !address.is_zero() {
            let actual_size = get_malloc_usable_size(address, is_offset_malloc);

            // If the side metadata for the address has not yet been mapped, we will map all the side metadata for the range [address, address + actual_size).
            if !is_meta_space_mapped(address, actual_size) {
                // Map the metadata space for the associated chunk
                self.map_metadata_and_update_bound(address, actual_size);
                // Update SFT
                assert!(crate::mmtk::SFT_MAP.has_sft_entry(address)); // make sure the address is okay with our SFT map
                unsafe { crate::mmtk::SFT_MAP.update(self, address, actual_size) };
            }

            // Set page marks for current object
            self.set_page_mark(address, actual_size);
            self.active_bytes.fetch_add(actual_size, Ordering::SeqCst);

            if is_offset_malloc {
                set_offset_malloc_bit(address);
            }

            #[cfg(debug_assertions)]
            if ASSERT_ALLOCATION {
                debug_assert!(actual_size != 0);
                self.active_mem.lock().unwrap().insert(address, actual_size);
            }
        }

        address
    }

    pub fn free(&self, addr: Address) {
        let offset_malloc_bit = is_offset_malloc(addr);
        let bytes = get_malloc_usable_size(addr, offset_malloc_bit);
        self.free_internal(addr, bytes, offset_malloc_bit);
    }

    // XXX optimize: We pass the bytes in to free as otherwise there were multiple
    // indirect call instructions in the generated assembly
    fn free_internal(&self, addr: Address, bytes: usize, offset_malloc_bit: bool) {
        if offset_malloc_bit {
            trace!("Free memory {:x}", addr);
            offset_free(addr);
            unsafe { unset_offset_malloc_bit_unsafe(addr) };
        } else {
            let ptr = addr.to_mut_ptr();
            trace!("Free memory {:?}", ptr);
            unsafe {
                free(ptr);
            }
        }

        self.active_bytes.fetch_sub(bytes, Ordering::SeqCst);

        #[cfg(debug_assertions)]
        if ASSERT_ALLOCATION {
            self.active_mem.lock().unwrap().insert(addr, 0).unwrap();
        }
    }

    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(!object.is_null());

        assert!(
            self.in_space(object),
            "Cannot mark an object {} that was not alloced by malloc.",
            object,
        );

        if !is_marked::<VM>(object, Ordering::Relaxed) {
            let chunk_start = conversions::chunk_align_down(object.to_object_start::<VM>());
            set_mark_bit::<VM>(object, Ordering::SeqCst);
            set_chunk_mark(chunk_start);
            queue.enqueue(object);
        }

        object
    }

    fn map_metadata_and_update_bound(&self, addr: Address, size: usize) {
        // Map the metadata space for the range [addr, addr + size)
        map_meta_space(&self.metadata, addr, size);

        // Update the bounds of the max and min chunk addresses seen -- this is used later in the sweep
        // Lockless compare-and-swap loops perform better than a locking variant

        // Update chunk_addr_min, basing on the start of the allocation: addr.
        {
            let min_chunk_start = conversions::chunk_align_down(addr);
            let mut min = self.chunk_addr_min.load(Ordering::Relaxed);
            while min_chunk_start < min {
                match self.chunk_addr_min.compare_exchange_weak(
                    min,
                    min_chunk_start,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(x) => min = x,
                }
            }
        }

        // Update chunk_addr_max, basing on the end of the allocation: addr + size.
        {
            let max_chunk_start = conversions::chunk_align_down(addr + size);
            let mut max = self.chunk_addr_max.load(Ordering::Relaxed);
            while max_chunk_start > max {
                match self.chunk_addr_max.compare_exchange_weak(
                    max,
                    max_chunk_start,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(x) => max = x,
                }
            }
        }
    }

    pub fn prepare(&mut self) {}

    pub fn release(&mut self) {
        use crate::scheduler::WorkBucketStage;
        let mut work_packets: Vec<Box<dyn GCWork<VM>>> = vec![];
        let mut chunk = self.chunk_addr_min.load(Ordering::Relaxed);
        let end = self.chunk_addr_max.load(Ordering::Relaxed) + BYTES_IN_CHUNK;

        // Since only a single thread generates the sweep work packets as well as it is a Stop-the-World collector,
        // we can assume that the chunk mark metadata is not being accessed by anything else and hence we use
        // non-atomic accesses
        let space = unsafe { &*(self as *const Self) };
        while chunk < end {
            if is_chunk_mapped(chunk) && unsafe { is_chunk_marked_unsafe(chunk) } {
                work_packets.push(Box::new(MSSweepChunk { ms: space, chunk }));
            }

            chunk += BYTES_IN_CHUNK;
        }

        debug!("Generated {} sweep work packets", work_packets.len());
        #[cfg(debug_assertions)]
        {
            self.total_work_packets
                .store(work_packets.len() as u32, Ordering::SeqCst);
            self.completed_work_packets.store(0, Ordering::SeqCst);
            self.work_live_bytes.store(0, Ordering::SeqCst);
        }

        self.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
    }

    pub fn sweep_chunk(&self, chunk_start: Address) {
        // Call the relevant sweep function depending on the location of the mark bits
        match *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
            MetadataSpec::OnSide(local_mark_bit_side_spec) => {
                self.sweep_chunk_mark_on_side(chunk_start, local_mark_bit_side_spec);
            }
            _ => {
                self.sweep_chunk_mark_in_header(chunk_start);
            }
        }
    }

    /// Given an object in MallocSpace, return its malloc address, whether it is an offset malloc, and malloc size
    fn get_malloc_addr_size(object: ObjectReference) -> (Address, bool, usize) {
        let obj_start = object.to_object_start::<VM>();
        let offset_malloc_bit = is_offset_malloc(obj_start);
        let bytes = get_malloc_usable_size(obj_start, offset_malloc_bit);
        (obj_start, offset_malloc_bit, bytes)
    }

    /// Clean up for an empty chunk
    fn clean_up_empty_chunk(&self, chunk_start: Address) {
        // Since the chunk mark metadata is a byte, we don't need synchronization
        unsafe { unset_chunk_mark_unsafe(chunk_start) };
        // Clear the SFT entry
        unsafe { crate::mmtk::SFT_MAP.clear(chunk_start) };
        // Clear the page marks - we are the only GC thread that is accessing this chunk
        unsafe { self.unset_page_mark(chunk_start, BYTES_IN_CHUNK) };
    }

    /// Sweep an object if it is dead, and unset page marks for empty pages before this object.
    /// Return true if the object is swept.
    fn sweep_object(&self, object: ObjectReference, empty_page_start: &mut Address) -> bool {
        let (obj_start, offset_malloc, bytes) = Self::get_malloc_addr_size(object);

        // We are the only thread that is dealing with the object. We can use non-atomic methods for the metadata.
        if !unsafe { is_marked_unsafe::<VM>(object) } {
            // Dead object
            trace!("Object {} has been allocated but not marked", object);

            // Free object
            self.free_internal(obj_start, bytes, offset_malloc);
            trace!("free object {}", object);
            unsafe { unset_vo_bit_unsafe::<VM>(object) };

            true
        } else {
            // Live object that we have marked

            // Unset marks for free pages and update last_object_end
            if !empty_page_start.is_zero() {
                // unset marks for pages since last object
                let current_page = object
                    .to_object_start::<VM>()
                    .align_down(BYTES_IN_MALLOC_PAGE);
                if current_page > *empty_page_start {
                    // we are the only GC thread that is accessing this chunk
                    unsafe {
                        self.unset_page_mark(*empty_page_start, current_page - *empty_page_start)
                    };
                }
            }

            // Update last_object_end
            *empty_page_start = (obj_start + bytes).align_up(BYTES_IN_MALLOC_PAGE);

            false
        }
    }

    /// Used when each chunk is done. Only called in debug build.
    #[cfg(debug_assertions)]
    fn debug_sweep_chunk_done(&self, live_bytes_in_the_chunk: usize) {
        debug!(
            "Used bytes after releasing: {}",
            self.active_bytes.load(Ordering::SeqCst)
        );

        let completed_packets = self.completed_work_packets.fetch_add(1, Ordering::SeqCst) + 1;
        self.work_live_bytes
            .fetch_add(live_bytes_in_the_chunk, Ordering::SeqCst);

        if completed_packets == self.total_work_packets.load(Ordering::Relaxed) {
            trace!(
                "work_live_bytes = {}, live_bytes = {}, active_bytes = {}",
                self.work_live_bytes.load(Ordering::Relaxed),
                live_bytes_in_the_chunk,
                self.active_bytes.load(Ordering::Relaxed)
            );
            debug_assert_eq!(
                self.work_live_bytes.load(Ordering::Relaxed),
                self.active_bytes.load(Ordering::Relaxed)
            );
        }
    }

    /// This function is called when the mark bits sit on the side metadata.
    /// This has been optimized with the use of bulk loading and bulk zeroing of
    /// metadata.
    ///
    /// This function uses non-atomic accesses to side metadata (although these
    /// non-atomic accesses should not have race conditions associated with them)
    /// as well as calls libc functions (`malloc_usable_size()`, `free()`)
    fn sweep_chunk_mark_on_side(&self, chunk_start: Address, mark_bit_spec: SideMetadataSpec) {
        // We can do xor on bulk for mark bits and valid object bits. If the result is zero, that means
        // the objects in it are all alive (both valid object bit and mark bit is set), and we do not
        // need to do anything for the region. Otherwise, we will sweep each single object in the region.
        // Note: Enabling this would result in inaccurate page accounting. We disable this by default, and
        // we will sweep object one by one.
        const BULK_XOR_ON_MARK_BITS: bool = false;

        if BULK_XOR_ON_MARK_BITS {
            #[cfg(debug_assertions)]
            let mut live_bytes = 0;

            debug!("Check active chunk {:?}", chunk_start);
            let mut address = chunk_start;
            let chunk_end = chunk_start + BYTES_IN_CHUNK;

            debug_assert!(
                crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC.log_bytes_in_region
                    == mark_bit_spec.log_bytes_in_region,
                "VO-bit and mark-bit metadata have different minimum object sizes!"
            );

            // For bulk xor'ing 128-bit vectors on architectures with vector instructions
            // Each bit represents an object of LOG_MIN_OBJ_SIZE size
            let bulk_load_size: usize = 128
                * (1 << crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC
                    .log_bytes_in_region);

            // The start of a possibly empty page. This will be updated during the sweeping, and always points to the next page of last live objects.
            let mut empty_page_start = Address::ZERO;

            // Scan the chunk by every 'bulk_load_size' region.
            while address < chunk_end {
                let alloc_128: u128 = unsafe {
                    load128(
                        &crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC,
                        address,
                    )
                };
                let mark_128: u128 = unsafe { load128(&mark_bit_spec, address) };

                // Check if there are dead objects in the bulk loaded region
                if alloc_128 ^ mark_128 != 0 {
                    let end = address + bulk_load_size;

                    // We will do non atomic load on the VO bit, as this is the only thread that access the VO bit for a chunk.
                    // Linear scan through the bulk load region.
                    let bulk_load_scan = crate::util::linear_scan::ObjectIterator::<
                        VM,
                        MallocObjectSize<VM>,
                        false,
                    >::new(address, end);
                    for object in bulk_load_scan {
                        self.sweep_object(object, &mut empty_page_start);
                    }
                } else {
                    // TODO we aren't actually accounting for the case where an object is alive and spans
                    // a page boundary as we don't know what the object sizes are/what is alive in the bulk region
                    if alloc_128 != 0 {
                        empty_page_start = address + bulk_load_size;
                    }
                }

                // We have processed this bulk load memory. Step to the next.
                address += bulk_load_size;
                debug_assert!(address.is_aligned_to(bulk_load_size));
            }

            // Linear scan through the chunk, and add up all the live object sizes.
            // We have to do this as a separate pass, as in the above pass, we did not go through all the live objects
            #[cfg(debug_assertions)]
            {
                let chunk_linear_scan = crate::util::linear_scan::ObjectIterator::<
                    VM,
                    MallocObjectSize<VM>,
                    false,
                >::new(chunk_start, chunk_end);
                for object in chunk_linear_scan {
                    let (obj_start, _, bytes) = Self::get_malloc_addr_size(object);

                    if ASSERT_ALLOCATION {
                        debug_assert!(
                            self.active_mem.lock().unwrap().contains_key(&obj_start),
                            "Address {} with VO bit is not in active_mem",
                            obj_start
                        );
                        debug_assert_eq!(
                            self.active_mem.lock().unwrap().get(&obj_start),
                            Some(&bytes),
                            "Address {} size in active_mem does not match the size from malloc_usable_size",
                            obj_start
                        );
                    }

                    debug_assert!(
                        unsafe { is_marked_unsafe::<VM>(object) },
                        "Dead object = {} found after sweep",
                        object
                    );

                    live_bytes += bytes;
                }
            }

            // Clear all the mark bits
            mark_bit_spec.bzero_metadata(chunk_start, BYTES_IN_CHUNK);

            // If we never updated empty_page_start, the entire chunk is empty.
            if empty_page_start.is_zero() {
                self.clean_up_empty_chunk(chunk_start);
            }

            #[cfg(debug_assertions)]
            self.debug_sweep_chunk_done(live_bytes);
        } else {
            self.sweep_each_object_in_chunk(chunk_start);
        }
    }

    /// This sweep function is called when the mark bit sits in the object header
    ///
    /// This function uses non-atomic accesses to side metadata (although these
    /// non-atomic accesses should not have race conditions associated with them)
    /// as well as calls libc functions (`malloc_usable_size()`, `free()`)
    fn sweep_chunk_mark_in_header(&self, chunk_start: Address) {
        self.sweep_each_object_in_chunk(chunk_start)
    }

    fn sweep_each_object_in_chunk(&self, chunk_start: Address) {
        #[cfg(debug_assertions)]
        let mut live_bytes = 0;

        debug!("Check active chunk {:?}", chunk_start);

        // The start of a possibly empty page. This will be updated during the sweeping, and always points to the next page of last live objects.
        let mut empty_page_start = Address::ZERO;

        let chunk_linear_scan = crate::util::linear_scan::ObjectIterator::<
            VM,
            MallocObjectSize<VM>,
            false,
        >::new(chunk_start, chunk_start + BYTES_IN_CHUNK);

        for object in chunk_linear_scan {
            #[cfg(debug_assertions)]
            if ASSERT_ALLOCATION {
                let (obj_start, _, bytes) = Self::get_malloc_addr_size(object);
                debug_assert!(
                    self.active_mem.lock().unwrap().contains_key(&obj_start),
                    "Address {} with VO bit is not in active_mem",
                    obj_start
                );
                debug_assert_eq!(
                    self.active_mem.lock().unwrap().get(&obj_start),
                    Some(&bytes),
                    "Address {} size in active_mem does not match the size from malloc_usable_size",
                    obj_start
                );
            }

            let live = !self.sweep_object(object, &mut empty_page_start);
            if live {
                // Live object. Unset mark bit.
                // We should be the only thread that access this chunk, it is okay to use non-atomic store.
                unsafe { unset_mark_bit::<VM>(object) };

                #[cfg(debug_assertions)]
                {
                    // Accumulate live bytes
                    let (_, _, bytes) = Self::get_malloc_addr_size(object);
                    live_bytes += bytes;
                }
            }
        }

        // If we never updated empty_page_start, the entire chunk is empty.
        if empty_page_start.is_zero() {
            self.clean_up_empty_chunk(chunk_start);
        } else if empty_page_start < chunk_start + BYTES_IN_CHUNK {
            // This is for the edge case where we have a live object and then no other live
            // objects afterwards till the end of the chunk. For example consider chunk
            // 0x0-0x400000 where only one object at 0x100 is alive. We will unset page bits
            // for 0x0-0x100 but then not unset it for the pages after 0x100. This checks
            // if we have empty pages at the end of a chunk that needs to be cleared.
            unsafe {
                self.unset_page_mark(
                    empty_page_start,
                    chunk_start + BYTES_IN_CHUNK - empty_page_start,
                )
            };
        }

        #[cfg(debug_assertions)]
        self.debug_sweep_chunk_done(live_bytes);
    }
}

struct MallocObjectSize<VM>(PhantomData<VM>);
impl<VM: VMBinding> crate::util::linear_scan::LinearScanObjectSize for MallocObjectSize<VM> {
    fn size(object: ObjectReference) -> usize {
        let (_, _, bytes) = MallocSpace::<VM>::get_malloc_addr_size(object);
        bytes
    }
}

use crate::scheduler::GCWork;
use crate::MMTK;

/// Simple work packet that just sweeps a single chunk
pub struct MSSweepChunk<VM: VMBinding> {
    ms: &'static MallocSpace<VM>,
    // starting address of a chunk
    chunk: Address,
}

impl<VM: VMBinding> GCWork<VM> for MSSweepChunk<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.ms.sweep_chunk(self.chunk);
    }
}
