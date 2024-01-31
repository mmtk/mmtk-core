use crate::global_state::GlobalState;
use crate::plan::PlanConstraints;
use crate::scheduler::GCWorkScheduler;
use crate::util::conversions::*;
use crate::util::metadata::side_metadata::{
    SideMetadataContext, SideMetadataSanity, SideMetadataSpec,
};
use crate::util::Address;
use crate::util::ObjectReference;

use crate::util::heap::layout::vm_layout::{vm_layout, LOG_BYTES_IN_CHUNK};
use crate::util::heap::{PageResource, VMRequest};
use crate::util::options::Options;
use crate::vm::{ActivePlan, Collection};

use crate::util::constants::{LOG_BYTES_IN_MBYTE, LOG_BYTES_IN_PAGE};
use crate::util::conversions;
use crate::util::opaque_pointer::*;

use crate::mmtk::SFT_MAP;
#[cfg(debug_assertions)]
use crate::policy::sft::EMPTY_SFT_NAME;
use crate::policy::sft::SFT;
use crate::util::copy::*;
use crate::util::heap::gc_trigger::GCTrigger;
use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
use crate::util::heap::layout::Mmapper;
use crate::util::heap::layout::VMMap;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::heap::HeapMeta;
use crate::util::memory;
use crate::vm::VMBinding;

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::Mutex;

use downcast_rs::Downcast;

pub trait Space<VM: VMBinding>: 'static + SFT + Sync + Downcast {
    fn as_space(&self) -> &dyn Space<VM>;
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static);
    fn get_page_resource(&self) -> &dyn PageResource<VM>;

    /// Initialize entires in SFT map for the space. This is called when the Space object
    /// has a non-moving address, as we will use the address to set sft.
    /// Currently after we create a boxed plan, spaces in the plan have a non-moving address.
    fn initialize_sft(&self, sft_map: &mut dyn crate::policy::sft_map::SFTMap);

    /// A check for the obvious out-of-memory case: if the requested size is larger than
    /// the heap size, it is definitely an OOM. We would like to identify that, and
    /// allows the binding to deal with OOM. Without this check, we will attempt
    /// to allocate from the page resource. If the requested size is unrealistically large
    /// (such as `usize::MAX`), it breaks the assumptions of our implementation of
    /// page resource, vm map, etc. This check prevents that, and allows us to
    /// handle the OOM case.
    /// Each allocator that may request an arbitrary size should call this method before
    /// acquring memory from the space. For example, bump pointer allocator and large object
    /// allocator need to call this method. On the other hand, allocators that only allocate
    /// memory in fixed size blocks do not need to call this method.
    /// An allocator should call this method before doing any computation on the size to
    /// avoid arithmatic overflow. If we have to do computation in the allocation fastpath and
    /// overflow happens there, there is nothing we can do about it.
    /// Return a boolean to indicate if we will be out of memory, determined by the check.
    fn will_oom_on_acquire(&self, tls: VMThread, size: usize) -> bool {
        let max_pages = self.get_gc_trigger().policy.get_max_heap_size_in_pages();
        let requested_pages = size >> LOG_BYTES_IN_PAGE;
        if requested_pages > max_pages {
            VM::VMCollection::out_of_memory(
                tls,
                crate::util::alloc::AllocationError::HeapOutOfMemory,
            );
            return true;
        }
        false
    }

    fn acquire(&self, tls: VMThread, pages: usize) -> Address {
        trace!("Space.acquire, tls={:?}", tls);

        debug_assert!(
            !self.will_oom_on_acquire(tls, pages << LOG_BYTES_IN_PAGE),
            "The requested pages is larger than the max heap size. Is will_go_oom_on_acquire used before acquring memory?"
        );

        // Should we poll to attempt to GC?
        // - If tls is collector, we cannot attempt a GC.
        // - If gc is disabled, we cannot attempt a GC.
        let should_poll =
            VM::VMActivePlan::is_mutator(tls) && VM::VMCollection::is_collection_enabled();
        // Is a GC allowed here? If we should poll but are not allowed to poll, we will panic.
        // initialize_collection() has to be called so we know GC is initialized.
        let allow_gc = should_poll && self.common().global_state.is_initialized();

        trace!("Reserving pages");
        let pr = self.get_page_resource();
        let pages_reserved = pr.reserve_pages(pages);
        trace!("Pages reserved");
        trace!("Polling ..");

        if should_poll && self.get_gc_trigger().poll(false, Some(self.as_space())) {
            debug!("Collection required");
            assert!(allow_gc, "GC is not allowed here: collection is not initialized (did you call initialize_collection()?).");

            // Clear the request, and inform GC trigger about the pending allocation.
            pr.clear_request(pages_reserved);
            self.get_gc_trigger()
                .policy
                .on_pending_allocation(pages_reserved);

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

                    let map_sidemetadata = || {
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
                    };
                    let grow_space = || {
                        self.grow_space(res.start, bytes, res.new_chunk);
                    };

                    // The scope of the lock is important in terms of performance when we have many allocator threads.
                    if SFT_MAP.get_side_metadata().is_some() {
                        // If the SFT map uses side metadata, so we have to initialize side metadata first.
                        map_sidemetadata();
                        // then grow space, which will use the side metadata we mapped above
                        grow_space();
                        // then we can drop the lock after grow_space()
                        drop(lock);
                    } else {
                        // In normal cases, we can drop lock immediately after grow_space()
                        grow_space();
                        drop(lock);
                        // and map side metadata without holding the lock
                        map_sidemetadata();
                    }

                    // TODO: Concurrent zeroing
                    if self.common().zeroed {
                        memory::zero(res.start, bytes);
                    }

                    // Some assertions
                    {
                        // --- Assert the start of the allocated region ---
                        // The start address SFT should be correct.
                        debug_assert_eq!(SFT_MAP.get_checked(res.start).name(), self.get_name());
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
                        debug_assert_eq!(SFT_MAP.get_checked(last_byte).name(), self.get_name());
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

                    let gc_performed = self.get_gc_trigger().poll(true, Some(self.as_space()));
                    debug_assert!(gc_performed, "GC not performed when forced.");

                    // Clear the request, and inform GC trigger about the pending allocation.
                    pr.clear_request(pages_reserved);
                    self.get_gc_trigger()
                        .policy
                        .on_pending_allocation(pages_reserved);

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
        self.address_in_space(object.to_address::<VM>())
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
            debug_assert!(
                SFT_MAP.get_checked(start).name() != EMPTY_SFT_NAME,
                "In grow_space(start = {}, bytes = {}, new_chunk = {}), we have empty SFT entries (chunk for {} = {})",
                start,
                bytes,
                new_chunk,
                start,
                SFT_MAP.get_checked(start).name()
            );
            debug_assert!(
                SFT_MAP.get_checked(start + bytes - 1).name() != EMPTY_SFT_NAME,
                "In grow_space(start = {}, bytes = {}, new_chunk = {}), we have empty SFT entries (chunk for {} = {})",
                start,
                bytes,
                new_chunk,
                start + bytes - 1,
                SFT_MAP.get_checked(start + bytes - 1).name()
            );
        }

        if new_chunk {
            unsafe { SFT_MAP.update(self.as_sft(), start, bytes) };
        }
    }

    /// Ensure this space is marked as mapped -- used when the space is already
    /// mapped (e.g. for a vm image which is externally mmapped.)
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

        self.common()
            .mmapper
            .mark_as_mapped(self.common().start, self.common().extent);
    }

    fn reserved_pages(&self) -> usize {
        let data_pages = self.get_page_resource().reserved_pages();
        let meta_pages = self.common().metadata.calculate_reserved_pages(data_pages);
        data_pages + meta_pages
    }

    /// Return the number of physical pages available.
    fn available_physical_pages(&self) -> usize {
        self.get_page_resource().get_available_physical_pages()
    }

    fn get_name(&self) -> &'static str {
        self.common().name
    }

    fn common(&self) -> &CommonSpace<VM>;
    fn get_gc_trigger(&self) -> &GCTrigger<VM> {
        self.common().gc_trigger.as_ref()
    }

    fn release_multiple_pages(&mut self, start: Address);

    /// What copy semantic we should use for this space if we copy objects from this space.
    /// This is only needed for plans that use SFTProcessEdges
    fn set_copy_for_sft_trace(&mut self, _semantics: Option<CopySemantics>) {
        panic!("A copying space should override this method")
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

/// Print the VM map for a space.
/// Space needs to be object-safe, so it cannot have methods that use extra generic type paramters. So this method is placed outside the Space trait.
/// This method can be invoked on a &dyn Space (space.as_space() will return &dyn Space).
#[allow(unused)]
pub(crate) fn print_vm_map<VM: VMBinding>(
    space: &dyn Space<VM>,
    out: &mut impl std::fmt::Write,
) -> Result<(), std::fmt::Error> {
    let common = space.common();
    write!(out, "{} ", common.name)?;
    if common.immortal {
        write!(out, "I")?;
    } else {
        write!(out, " ")?;
    }
    if common.movable {
        write!(out, " ")?;
    } else {
        write!(out, "N")?;
    }
    write!(out, " ")?;
    if common.contiguous {
        write!(
            out,
            "{}->{}",
            common.start,
            common.start + common.extent - 1
        )?;
        match common.vmrequest {
            VMRequest::Extent { extent, .. } => {
                write!(out, " E {}", extent)?;
            }
            VMRequest::Fraction { frac, .. } => {
                write!(out, " F {}", frac)?;
            }
            _ => {}
        }
    } else {
        let mut a = space
            .get_page_resource()
            .common()
            .get_head_discontiguous_region();
        while !a.is_zero() {
            write!(
                out,
                "{}->{}",
                a,
                a + space.common().vm_map().get_contiguous_region_size(a) - 1
            )?;
            a = space.common().vm_map().get_next_contiguous_region(a);
            if !a.is_zero() {
                write!(out, " ")?;
            }
        }
    }
    writeln!(out)?;

    Ok(())
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

    pub vm_map: &'static dyn VMMap,
    pub mmapper: &'static dyn Mmapper,

    pub(crate) metadata: SideMetadataContext,

    /// This field equals to needs_log_bit in the plan constraints.
    // TODO: This should be a constant for performance.
    pub needs_log_bit: bool,

    /// A lock used during acquire() to make sure only one thread can allocate.
    pub acquire_lock: Mutex<()>,

    pub gc_trigger: Arc<GCTrigger<VM>>,
    pub global_state: Arc<GlobalState>,

    p: PhantomData<VM>,
}

/// Arguments passed from a policy to create a space. This includes policy specific args.
pub struct PolicyCreateSpaceArgs<'a, VM: VMBinding> {
    pub plan_args: PlanCreateSpaceArgs<'a, VM>,
    pub movable: bool,
    pub immortal: bool,
    pub local_side_metadata_specs: Vec<SideMetadataSpec>,
}

/// Arguments passed from a plan to create a space.
pub struct PlanCreateSpaceArgs<'a, VM: VMBinding> {
    pub name: &'static str,
    pub zeroed: bool,
    pub vmrequest: VMRequest,
    pub global_side_metadata_specs: Vec<SideMetadataSpec>,
    pub vm_map: &'static dyn VMMap,
    pub mmapper: &'static dyn Mmapper,
    pub heap: &'a mut HeapMeta,
    pub constraints: &'a PlanConstraints,
    pub gc_trigger: Arc<GCTrigger<VM>>,
    pub scheduler: Arc<GCWorkScheduler<VM>>,
    pub options: &'a Options,
    pub global_state: Arc<GlobalState>,
}

impl<'a, VM: VMBinding> PlanCreateSpaceArgs<'a, VM> {
    /// Turning PlanCreateSpaceArgs into a PolicyCreateSpaceArgs
    pub fn into_policy_args(
        self,
        movable: bool,
        immortal: bool,
        policy_metadata_specs: Vec<SideMetadataSpec>,
    ) -> PolicyCreateSpaceArgs<'a, VM> {
        PolicyCreateSpaceArgs {
            movable,
            immortal,
            local_side_metadata_specs: policy_metadata_specs,
            plan_args: self,
        }
    }
}

impl<VM: VMBinding> CommonSpace<VM> {
    pub fn new(args: PolicyCreateSpaceArgs<VM>) -> Self {
        let mut rtn = CommonSpace {
            name: args.plan_args.name,
            descriptor: SpaceDescriptor::UNINITIALIZED,
            vmrequest: args.plan_args.vmrequest,
            copy: None,
            immortal: args.immortal,
            movable: args.movable,
            contiguous: true,
            zeroed: args.plan_args.zeroed,
            start: unsafe { Address::zero() },
            extent: 0,
            vm_map: args.plan_args.vm_map,
            mmapper: args.plan_args.mmapper,
            needs_log_bit: args.plan_args.constraints.needs_log_bit,
            gc_trigger: args.plan_args.gc_trigger,
            metadata: SideMetadataContext {
                global: args.plan_args.global_side_metadata_specs,
                local: args.local_side_metadata_specs,
            },
            acquire_lock: Mutex::new(()),
            global_state: args.plan_args.global_state,
            p: PhantomData,
        };

        let vmrequest = args.plan_args.vmrequest;
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
                extent: _extent, ..
            } => (_extent, false),
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
            args.plan_args.heap.reserve(extent, top)
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

        // We only initialize our vm map if the range of the space is in our available heap range. For normally spaces,
        // they are definitely in our heap range. But for VM space, a runtime could give us an arbitrary range. We only
        // insert into our vm map if the range overlaps with our heap.
        {
            use crate::util::heap::layout;
            let overlap =
                Address::range_intersection(&(start..start + extent), &layout::available_range());
            if !overlap.is_empty() {
                args.plan_args.vm_map.insert(
                    overlap.start,
                    overlap.end - overlap.start,
                    rtn.descriptor,
                );
            }
        }

        // For contiguous space, we know its address range so we reserve metadata memory for its range.
        if rtn
            .metadata
            .try_map_metadata_address_range(rtn.start, rtn.extent)
            .is_err()
        {
            // TODO(Javad): handle meta space allocation failure
            panic!("failed to mmap meta memory");
        }

        debug!(
            "Created space {} [{}, {}) for {} bytes",
            rtn.name,
            start,
            start + extent,
            extent
        );

        rtn
    }

    pub fn initialize_sft(
        &self,
        sft: &(dyn SFT + Sync + 'static),
        sft_map: &mut dyn crate::policy::sft_map::SFTMap,
    ) {
        // We have to keep this for now: if a space is contiguous, our page resource will NOT consider newly allocated chunks
        // as new chunks (new_chunks = true). In that case, in grow_space(), we do not set SFT when new_chunks = false.
        // We can fix this by either of these:
        // * fix page resource, so it propelry returns new_chunk
        // * change grow_space() so it sets SFT no matter what the new_chunks value is.
        // FIXME: eagerly initializing SFT is not a good idea.
        if self.contiguous {
            unsafe { sft_map.eager_initialize(sft, self.start, self.extent) };
        }
    }

    pub fn vm_map(&self) -> &'static dyn VMMap {
        self.vm_map
    }
}

fn get_frac_available(frac: f32) -> usize {
    trace!("AVAILABLE_START={}", vm_layout().available_start());
    trace!("AVAILABLE_END={}", vm_layout().available_end());
    let bytes = (frac * vm_layout().available_bytes() as f32) as usize;
    trace!("bytes={}*{}={}", frac, vm_layout().available_bytes(), bytes);
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
