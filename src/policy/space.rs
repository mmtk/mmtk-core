use crate::util::conversions::*;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::Address;
use crate::util::ObjectReference;

use crate::util::heap::layout::vm_layout_constants::{AVAILABLE_BYTES, LOG_BYTES_IN_CHUNK};
use crate::util::heap::layout::vm_layout_constants::{AVAILABLE_END, AVAILABLE_START};
use crate::util::heap::{PageResource, VMRequest};
use crate::vm::{ActivePlan, Collection, ObjectModel};

use crate::util::constants::LOG_BYTES_IN_MBYTE;
use crate::util::conversions;
use crate::util::opaque_pointer::*;

use crate::mmtk::SFT_MAP;
#[cfg(debug_assertions)]
use crate::policy::sft::EMPTY_SFT_NAME;
use crate::policy::sft::SFT;
use crate::policy::sft_map::SFTMap;
use crate::util::copy::*;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::map::Map;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::layout::Mmapper as IMmapper;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::heap::HeapMeta;
use crate::util::memory;

use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::Mutex;

use downcast_rs::Downcast;

pub trait Space<VM: VMBinding>: 'static + SFT + Sync + Downcast {
    fn as_space(&self) -> &dyn Space<VM>;
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static);
    fn get_page_resource(&self) -> &dyn PageResource<VM>;
    fn init(&mut self, vm_map: &'static VMMap);

    fn acquire(&self, tls: VMThread, pages: usize) -> Address {
        trace!("Space.acquire, tls={:?}", tls);
        // Should we poll to attempt to GC?
        // - If tls is collector, we cannot attempt a GC.
        // - If gc is disabled, we cannot attempt a GC.
        let should_poll = VM::VMActivePlan::is_mutator(tls)
            && VM::VMActivePlan::global().should_trigger_gc_when_heap_is_full();
        // Is a GC allowed here? If we should poll but are not allowed to poll, we will panic.
        // initialize_collection() has to be called so we know GC is initialized.
        let allow_gc = should_poll && VM::VMActivePlan::global().is_initialized();

        trace!("Reserving pages");
        let pr = self.get_page_resource();
        let pages_reserved = pr.reserve_pages(pages);
        trace!("Pages reserved");
        trace!("Polling ..");

        if should_poll && VM::VMActivePlan::global().poll(false, Some(self.as_space())) {
            debug!("Collection required");
            assert!(allow_gc, "GC is not allowed here: collection is not initialized (did you call initialize_collection()?).");
            pr.clear_request(pages_reserved);
            VM::VMCollection::block_for_gc(VMMutatorThread(tls)); // We have checked that this is mutator
            unsafe { Address::zero() }
        } else {
            debug!("Collection not required");

            // We need this lock: Othrewise, it is possible that one thread acquires pages in a new chunk, but not yet
            // set SFT for it (in grow_space()), and another thread acquires pages in the same chunk, which is not
            // a new chunk so grow_space() won't be called on it. The second thread could return a result in the chunk before
            // its SFT is properly set.
            // We need to minimize the scope of this lock for performance when we have many threads (mutator threads, or GC threads with copying allocators).
            // See: https://github.com/mmtk/mmtk-core/issues/610
            let lock = self.common().acquire_lock.lock().unwrap();

            match pr.get_new_pages(self.common().descriptor, pages_reserved, pages, tls) {
                Ok(res) => {
                    debug!(
                        "Got new pages {} ({} pages) for {} in chunk {}, new_chunk? {}",
                        res.start,
                        res.pages,
                        self.get_name(),
                        conversions::chunk_align_down(res.start),
                        res.new_chunk
                    );
                    let bytes = conversions::pages_to_bytes(res.pages);
                    self.grow_space(res.start, bytes, res.new_chunk);

                    // Once we finish grow_space, we can drop the lock.
                    drop(lock);

                    // Mmap the pages and the side metadata, and handle error. In case of any error,
                    // we will either call back to the VM for OOM, or simply panic.
                    if let Err(mmap_error) = self
                        .common()
                        .mmapper
                        .ensure_mapped(res.start, res.pages)
                        .and(
                            self.common()
                                .metadata
                                .try_map_metadata_space(res.start, bytes),
                        )
                    {
                        memory::handle_mmap_error::<VM>(mmap_error, tls);
                    }

                    // TODO: Concurrent zeroing
                    if self.common().zeroed {
                        memory::zero(res.start, bytes);
                    }

                    // Some assertions
                    {
                        // --- Assert the start of the allocated region ---
                        // The start address SFT should be correct.
                        debug_assert_eq!(SFT_MAP.get(res.start).name(), self.get_name());
                        // The start address is in our space.
                        debug_assert!(self.address_in_space(res.start));
                        // The descriptor should be correct.
                        debug_assert_eq!(
                            self.common().vm_map().get_descriptor_for_address(res.start),
                            self.common().descriptor
                        );

                        // --- Assert the last byte in the allocated region ---
                        let last_byte = res.start + bytes - 1;
                        // The SFT for the last byte in the allocated memory should be correct.
                        debug_assert_eq!(SFT_MAP.get(last_byte).name(), self.get_name());
                        // The last byte in the allocated memory should be in this space.
                        debug_assert!(self.address_in_space(last_byte));
                        // The descriptor for the last byte should be correct.
                        debug_assert_eq!(
                            self.common().vm_map().get_descriptor_for_address(last_byte),
                            self.common().descriptor
                        );
                    }

                    debug!("Space.acquire(), returned = {}", res.start);
                    res.start
                }
                Err(_) => {
                    drop(lock); // drop the lock immediately

                    // We thought we had memory to allocate, but somehow failed the allocation. Will force a GC.
                    assert!(
                        allow_gc,
                        "Physical allocation failed when GC is not allowed!"
                    );

                    let gc_performed = VM::VMActivePlan::global().poll(true, Some(self.as_space()));
                    debug_assert!(gc_performed, "GC not performed when forced.");
                    pr.clear_request(pages_reserved);
                    VM::VMCollection::block_for_gc(VMMutatorThread(tls)); // We asserted that this is mutator.
                    unsafe { Address::zero() }
                }
            }
        }
    }

    fn address_in_space(&self, start: Address) -> bool {
        if !self.common().descriptor.is_contiguous() {
            self.common().vm_map().get_descriptor_for_address(start) == self.common().descriptor
        } else {
            start >= self.common().start && start < self.common().start + self.common().extent
        }
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        let start = VM::VMObjectModel::ref_to_address(object);
        self.address_in_space(start)
    }

    /**
     * This is called after we get result from page resources.  The space may
     * tap into the hook to monitor heap growth.  The call is made from within the
     * page resources' critical region, immediately before yielding the lock.
     *
     * @param start The start of the newly allocated space
     * @param bytes The size of the newly allocated space
     * @param new_chunk {@code true} if the new space encroached upon or started a new chunk or chunks.
     */
    fn grow_space(&self, start: Address, bytes: usize, new_chunk: bool) {
        trace!(
            "Grow space from {} for {} bytes (new chunk = {})",
            start,
            bytes,
            new_chunk
        );

        // If this is not a new chunk, the SFT for [start, start + bytes) should alreayd be initialized.
        #[cfg(debug_assertions)]
        if !new_chunk {
            debug_assert!(SFT_MAP.get(start).name() != EMPTY_SFT_NAME, "In grow_space(start = {}, bytes = {}, new_chunk = {}), we have empty SFT entries (chunk for {} = {})", start, bytes, new_chunk, start, SFT_MAP.get(start).name());
            debug_assert!(SFT_MAP.get(start + bytes - 1).name() != EMPTY_SFT_NAME, "In grow_space(start = {}, bytes = {}, new_chunk = {}), we have empty SFT entries (chunk for {} = {}", start, bytes, new_chunk, start + bytes - 1, SFT_MAP.get(start + bytes - 1).name());
        }

        if new_chunk {
            SFT_MAP.update(self.as_sft(), start, bytes);
        }
    }

    /**
     *  Ensure this space is marked as mapped -- used when the space is already
     *  mapped (e.g. for a vm image which is externally mmapped.)
     */
    fn ensure_mapped(&self) {
        if self
            .common()
            .metadata
            .try_map_metadata_space(self.common().start, self.common().extent)
            .is_err()
        {
            // TODO(Javad): handle meta space allocation failure
            panic!("failed to mmap meta memory");
        }
        SFT_MAP.update(self.as_sft(), self.common().start, self.common().extent);
        use crate::util::heap::layout::mmapper::Mmapper;
        self.common()
            .mmapper
            .mark_as_mapped(self.common().start, self.common().extent);
    }

    fn reserved_pages(&self) -> usize {
        let data_pages = self.get_page_resource().reserved_pages();
        let meta_pages = self.common().metadata.calculate_reserved_pages(data_pages);
        data_pages + meta_pages
    }

    fn get_name(&self) -> &'static str {
        self.common().name
    }

    fn common(&self) -> &CommonSpace<VM>;

    fn release_multiple_pages(&mut self, start: Address);

    /// What copy semantic we should use for this space if we copy objects from this space.
    /// This is only needed for plans that use SFTProcessEdges
    fn set_copy_for_sft_trace(&mut self, _semantics: Option<CopySemantics>) {
        panic!("A copying space should override this method")
    }

    fn print_vm_map(&self) {
        let common = self.common();
        print!("{} ", common.name);
        if common.immortal {
            print!("I");
        } else {
            print!(" ");
        }
        if common.movable {
            print!(" ");
        } else {
            print!("N");
        }
        print!(" ");
        if common.contiguous {
            print!("{}->{}", common.start, common.start + common.extent - 1);
            match common.vmrequest {
                VMRequest::Extent { extent, .. } => {
                    print!(" E {}", extent);
                }
                VMRequest::Fraction { frac, .. } => {
                    print!(" F {}", frac);
                }
                _ => {}
            }
        } else {
            let mut a = self
                .get_page_resource()
                .common()
                .get_head_discontiguous_region();
            while !a.is_zero() {
                print!(
                    "{}->{}",
                    a,
                    a + self.common().vm_map().get_contiguous_region_size(a) - 1
                );
                a = self.common().vm_map().get_next_contiguous_region(a);
                if !a.is_zero() {
                    print!(" ");
                }
            }
        }
        println!();
    }

    /// Ensure that the current space's metadata context does not have any issues.
    /// Panics with a suitable message if any issue is detected.
    /// It also initialises the sanity maps which will then be used if the `extreme_assertions` feature is active.
    /// Internally this calls verify_metadata_context() from `util::metadata::sanity`
    ///
    /// This function is called once per space by its parent plan but may be called multiple times per policy.
    ///
    /// Arguments:
    /// * `side_metadata_sanity_checker`: The `SideMetadataSanity` object instantiated in the calling plan.
    fn verify_side_metadata_sanity(&self, side_metadata_sanity_checker: &mut SideMetadataSanity) {
        side_metadata_sanity_checker
            .verify_metadata_context(std::any::type_name::<Self>(), &self.common().metadata)
    }
}

impl_downcast!(Space<VM> where VM: VMBinding);

pub struct CommonSpace<VM: VMBinding> {
    pub name: &'static str,
    pub descriptor: SpaceDescriptor,
    pub vmrequest: VMRequest,

    /// For a copying space that allows sft_trace_object(), this should be set before each GC so we know
    // the copy semantics for the space.
    pub copy: Option<CopySemantics>,

    immortal: bool,
    movable: bool,
    pub contiguous: bool,
    pub zeroed: bool,

    pub start: Address,
    pub extent: usize,
    pub head_discontiguous_region: Address,

    pub vm_map: &'static VMMap,
    pub mmapper: &'static Mmapper,

    pub metadata: SideMetadataContext,

    /// This field equals to needs_log_bit in the plan constraints.
    // TODO: This should be a constant for performance.
    pub needs_log_bit: bool,

    /// A lock used during acquire() to make sure only one thread can allocate.
    pub acquire_lock: Mutex<()>,

    p: PhantomData<VM>,
}

pub struct SpaceOptions {
    pub name: &'static str,
    pub movable: bool,
    pub immortal: bool,
    pub zeroed: bool,
    pub needs_log_bit: bool,
    pub vmrequest: VMRequest,
    pub side_metadata_specs: SideMetadataContext,
}

/// Print debug info for SFT. Should be false when committed.
const DEBUG_SPACE: bool = cfg!(debug_assertions) && false;

impl<VM: VMBinding> CommonSpace<VM> {
    pub fn new(
        opt: SpaceOptions,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        let mut rtn = CommonSpace {
            name: opt.name,
            descriptor: SpaceDescriptor::UNINITIALIZED,
            vmrequest: opt.vmrequest,
            copy: None,
            immortal: opt.immortal,
            movable: opt.movable,
            contiguous: true,
            zeroed: opt.zeroed,
            start: unsafe { Address::zero() },
            extent: 0,
            head_discontiguous_region: unsafe { Address::zero() },
            vm_map,
            mmapper,
            needs_log_bit: opt.needs_log_bit,
            metadata: opt.side_metadata_specs,
            p: PhantomData,
            acquire_lock: Mutex::new(()),
        };

        let vmrequest = opt.vmrequest;
        if vmrequest.is_discontiguous() {
            rtn.contiguous = false;
            // FIXME
            rtn.descriptor = SpaceDescriptor::create_descriptor();
            // VM.memory.setHeapRange(index, HEAP_START, HEAP_END);
            return rtn;
        }

        let (extent, top) = match vmrequest {
            VMRequest::Fraction { frac, top: _top } => (get_frac_available(frac), _top),
            VMRequest::Extent {
                extent: _extent,
                top: _top,
            } => (_extent, _top),
            VMRequest::Fixed {
                extent: _extent,
                top: _top,
                ..
            } => (_extent, _top),
            _ => unreachable!(),
        };

        assert!(
            extent == raw_align_up(extent, BYTES_IN_CHUNK),
            "{} requested non-aligned extent: {} bytes",
            rtn.name,
            extent
        );

        let start = if let VMRequest::Fixed { start: _start, .. } = vmrequest {
            _start
        } else {
            // FIXME
            //if (HeapLayout.vmMap.isFinalized()) VM.assertions.fail("heap is narrowed after regionMap is finalized: " + name);
            heap.reserve(extent, top)
        };
        assert!(
            start == chunk_align_up(start),
            "{} starting on non-aligned boundary: {}",
            rtn.name,
            start
        );

        rtn.contiguous = true;
        rtn.start = start;
        rtn.extent = extent;
        // FIXME
        rtn.descriptor = SpaceDescriptor::create_descriptor_from_heap_range(start, start + extent);
        // VM.memory.setHeapRange(index, start, start.plus(extent));
        vm_map.insert(start, extent, rtn.descriptor);

        if DEBUG_SPACE {
            println!(
                "Created space {} [{}, {}) for {} bytes",
                rtn.name,
                start,
                start + extent,
                extent
            );
        }

        rtn
    }

    pub fn init(&self, space: &dyn Space<VM>) {
        // For contiguous space, we eagerly initialize SFT map based on its address range.
        if self.contiguous {
            if self
                .metadata
                .try_map_metadata_address_range(self.start, self.extent)
                .is_err()
            {
                // TODO(Javad): handle meta space allocation failure
                panic!("failed to mmap meta memory");
            }
            SFT_MAP.update(space.as_sft(), self.start, self.extent);
        }
    }

    pub fn vm_map(&self) -> &'static VMMap {
        self.vm_map
    }
}

fn get_frac_available(frac: f32) -> usize {
    trace!("AVAILABLE_START={}", AVAILABLE_START);
    trace!("AVAILABLE_END={}", AVAILABLE_END);
    let bytes = (frac * AVAILABLE_BYTES as f32) as usize;
    trace!("bytes={}*{}={}", frac, AVAILABLE_BYTES, bytes);
    let mb = bytes >> LOG_BYTES_IN_MBYTE;
    let rtn = mb << LOG_BYTES_IN_MBYTE;
    trace!("rtn={}", rtn);
    let aligned_rtn = raw_align_up(rtn, BYTES_IN_CHUNK);
    trace!("aligned_rtn={}", aligned_rtn);
    aligned_rtn
}

pub fn required_chunks(pages: usize) -> usize {
    let extent = raw_align_up(pages_to_bytes(pages), BYTES_IN_CHUNK);
    extent >> LOG_BYTES_IN_CHUNK
}
