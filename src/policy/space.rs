use crate::plan::VectorObjectQueue;
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
use crate::scheduler::GCWorker;
use crate::util::copy::*;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::map::Map;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::layout::vm_layout_constants::MAX_CHUNKS;
use crate::util::heap::layout::Mmapper as IMmapper;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::heap::HeapMeta;
use crate::util::memory;

#[cfg(feature = "is_mmtk_object")]
use crate::util::alloc_bit;

use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::Mutex;

use downcast_rs::Downcast;

/// Space Function Table (SFT).
///
/// This trait captures functions that reflect _space-specific per-object
/// semantics_.   These functions are implemented for each object via a special
/// space-based dynamic dispatch mechanism where the semantics are _not_
/// determined by the object's _type_, but rather, are determined by the _space_
/// that the object is in.
///
/// The underlying mechanism exploits the fact that spaces use the address space
/// at an MMTk chunk granularity with the consequence that each chunk maps to
/// exactluy one space, so knowing the chunk for an object reveals its space.
/// The dispatch then works by performing simple address arithmetic on the object
/// reference to find a chunk index which is used to index a table which returns
/// the space.   The relevant function is then dispatched against that space
/// object.
///
/// We use the SFT trait to simplify typing for Rust, so our table is a
/// table of SFT rather than Space.
pub trait SFT {
    /// The space name
    fn name(&self) -> &str;
    /// Get forwarding pointer if the object is forwarded.
    #[inline(always)]
    fn get_forwarded_object(&self, _object: ObjectReference) -> Option<ObjectReference> {
        None
    }
    /// Is the object live, determined by the policy?
    fn is_live(&self, object: ObjectReference) -> bool;
    /// Is the object reachable, determined by the policy?
    /// Note: Objects in ImmortalSpace may have `is_live = true` but are actually unreachable.
    #[inline(always)]
    fn is_reachable(&self, object: ObjectReference) -> bool {
        self.is_live(object)
    }
    /// Is the object movable, determined by the policy? E.g. the policy is non-moving,
    /// or the object is pinned.
    fn is_movable(&self) -> bool;
    /// Is the object sane? A policy should return false if there is any abnormality about
    /// object - the sanity checker will fail if an object is not sane.
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool;
    /// Is the object managed by MMTk? For most cases, if we find the sft for an object, that means
    /// the object is in the space and managed by MMTk. However, for some spaces, like MallocSpace,
    /// we mark the entire chunk in the SFT table as a malloc space, but only some of the addresses
    /// in the space contain actual MMTk objects. So they need a further check.
    #[inline(always)]
    fn is_in_space(&self, _object: ObjectReference) -> bool {
        true
    }
    /// Is `addr` a valid object reference to an object allocated in this space?
    /// This default implementation works for all spaces that use MMTk's mapper to allocate memory.
    /// Some spaces, like `MallocSpace`, use third-party libraries to allocate memory.
    /// Such spaces needs to override this method.
    #[cfg(feature = "is_mmtk_object")]
    #[inline(always)]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        // Having found the SFT means the `addr` is in one of our spaces.
        // Although the SFT map is allocated eagerly when the space is contiguous,
        // the pages of the space itself are acquired on demand.
        // Therefore, the page of `addr` may not have been mapped, yet.
        if !addr.is_mapped() {
            return false;
        }
        // The `addr` is mapped. We use the global alloc bit to get the exact answer.
        alloc_bit::is_alloced_object(addr)
    }
    /// Initialize object metadata (in the header, or in the side metadata).
    fn initialize_object_metadata(&self, object: ObjectReference, alloc: bool);
    /// Trace objects through SFT. This along with [`SFTProcessEdges`](mmtk/scheduler/gc_work/SFTProcessEdges)
    /// provides an easy way for most plans to trace objects without the need to implement any plan-specific
    /// code. However, tracing objects for some policies are more complicated, and they do not provide an
    /// implementation of this method. For example, mark compact space requires trace twice in each GC.
    /// Immix has defrag trace and fast trace.
    fn sft_trace_object(
        &self,
        // We use concrete type for `queue` because SFT doesn't support generic parameters,
        // and SFTProcessEdges uses `VectorObjectQueue`.
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        worker: GCWorkerMutRef,
    ) -> ObjectReference;
}

// Create erased VM refs for these types that will be used in `sft_trace_object()`.
// In this way, we can store the refs with <VM> in SFT (which cannot have parameters with generic type parameters)

use crate::util::erase_vm::define_erased_vm_mut_ref;
define_erased_vm_mut_ref!(GCWorkerMutRef = GCWorker<VM>);

/// Print debug info for SFT. Should be false when committed.
const DEBUG_SFT: bool = cfg!(debug_assertions) && false;

#[derive(Debug)]
struct EmptySpaceSFT {}

const EMPTY_SFT_NAME: &str = "empty";

impl SFT for EmptySpaceSFT {
    fn name(&self) -> &str {
        EMPTY_SFT_NAME
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        panic!(
            "Called is_live() on {:x}, which maps to an empty space",
            object
        )
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        warn!("Object in empty space!");
        false
    }
    fn is_movable(&self) -> bool {
        /*
         * FIXME steveb I think this should panic (ie the function should not
         * be invoked on an empty space).   However, JikesRVM currently does
         * call this in an unchecked way and expects 'false' for out of bounds
         * addresses.  So until that is fixed upstream, we'll return false here.
         *
         * panic!("called is_movable() on empty space")
         */
        false
    }
    #[inline(always)]
    fn is_in_space(&self, _object: ObjectReference) -> bool {
        false
    }
    #[cfg(feature = "is_mmtk_object")]
    #[inline(always)]
    fn is_mmtk_object(&self, _addr: Address) -> bool {
        false
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        panic!(
            "Called initialize_object_metadata() on {:x}, which maps to an empty space",
            object
        )
    }

    fn sft_trace_object(
        &self,
        _queue: &mut VectorObjectQueue,
        _object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        // We do not have the `VM` type parameter here, so we cannot forward the call to the VM.
        panic!(
            "Call trace_object() on {} (chunk {}), which maps to an empty space. SFTProcessEdges does not support the fallback to vm_trace_object().",
            _object,
            conversions::chunk_align_down(_object.to_address()),
        )
    }
}

pub struct SFTMap<'a> {
    sft: Vec<&'a (dyn SFT + Sync + 'static)>,
}

// TODO: MMTK<VM> holds a reference to SFTMap. We should have a safe implementation rather than use raw pointers for dyn SFT.
unsafe impl<'a> Sync for SFTMap<'a> {}

const EMPTY_SPACE_SFT: EmptySpaceSFT = EmptySpaceSFT {};

impl<'a> SFTMap<'a> {
    pub fn new() -> Self {
        SFTMap {
            sft: vec![&EMPTY_SPACE_SFT; MAX_CHUNKS],
        }
    }
    // This is a temporary solution to allow unsafe mut reference. We do not want several occurrence
    // of the same unsafe code.
    // FIXME: We need a safe implementation.
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    pub fn get(&self, address: Address) -> &'a dyn SFT {
        debug_assert!(address.chunk_index() < MAX_CHUNKS);
        let res = unsafe { *self.sft.get_unchecked(address.chunk_index()) };
        if DEBUG_SFT {
            trace!(
                "Get SFT for {} #{} = {}",
                address,
                address.chunk_index(),
                res.name()
            );
        }
        res
    }

    fn log_update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize) {
        debug!("Update SFT for Chunk {} as {}", start, space.name(),);
        let first = start.chunk_index();
        let start_chunk = chunk_index_to_address(first);
        debug!(
            "Update SFT for {} bytes of Chunk {} #{}",
            bytes, start_chunk, first
        );
    }

    fn trace_sft_map(&self) {
        trace!("{}", self.print_sft_map());
    }

    // This can be used during debugging to print SFT map.
    fn print_sft_map(&self) -> String {
        // print the entire SFT map
        let mut res = String::new();

        const SPACE_PER_LINE: usize = 10;
        for i in (0..self.sft.len()).step_by(SPACE_PER_LINE) {
            let max = if i + SPACE_PER_LINE > self.sft.len() {
                self.sft.len()
            } else {
                i + SPACE_PER_LINE
            };
            let chunks: Vec<usize> = (i..max).collect();
            let space_names: Vec<&str> = chunks.iter().map(|&x| self.sft[x].name()).collect();
            res.push_str(&format!(
                "{}: {}",
                chunk_index_to_address(i),
                space_names.join(",")
            ));
            res.push('\n');
        }

        res
    }

    /// Update SFT map for the given address range.
    /// It should be used when we acquire new memory and use it as part of a space. For example, the cases include:
    /// 1. when a space grows, 2. when initializing a contiguous space, 3. when ensure_mapped() is called on a space.
    pub fn update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize) {
        if DEBUG_SFT {
            self.log_update(space, start, bytes);
        }
        let first = start.chunk_index();
        let last = conversions::chunk_align_up(start + bytes).chunk_index();
        for chunk in first..last {
            self.set(chunk, space);
        }
        if DEBUG_SFT {
            self.trace_sft_map();
        }
    }

    // TODO: We should clear a SFT entry when a space releases a chunk.
    #[allow(dead_code)]
    pub fn clear(&self, chunk_start: Address) {
        if DEBUG_SFT {
            debug!(
                "Clear SFT for chunk {} (was {})",
                chunk_start,
                self.get(chunk_start).name()
            );
        }
        assert!(chunk_start.is_aligned_to(BYTES_IN_CHUNK));
        let chunk_idx = chunk_start.chunk_index();
        self.set(chunk_idx, &EMPTY_SPACE_SFT);
    }

    // Currently only used by 32 bits vm map
    #[allow(dead_code)]
    pub fn clear_by_index(&self, chunk_idx: usize) {
        if DEBUG_SFT {
            let chunk_start = chunk_index_to_address(chunk_idx);
            debug!(
                "Clear SFT for chunk {} by index (was {})",
                chunk_start,
                self.get(chunk_start).name()
            );
        }
        self.set(chunk_idx, &EMPTY_SPACE_SFT)
    }

    fn set(&self, chunk: usize, sft: &(dyn SFT + Sync + 'static)) {
        /*
         * This is safe (only) because a) this is only called during the
         * allocation and deallocation of chunks, which happens under a global
         * lock, and b) it only transitions from empty to valid and valid to
         * empty, so if there were a race to view the contents, in the one case
         * it would either see the new (valid) space or an empty space (both of
         * which are reasonable), and in the other case it would either see the
         * old (valid) space or an empty space, both of which are valid.
         */
        let self_mut = unsafe { self.mut_self() };
        // It is okay to set empty to valid, or set valid to empty. It is wrong if we overwrite a valid value with another valid value.
        if cfg!(debug_assertions) {
            let old = self_mut.sft[chunk].name();
            let new = sft.name();
            // Allow overwriting the same SFT pointer. E.g., if we have set SFT map for a space, then ensure_mapped() is called on the same,
            // in which case, we still set SFT map again.
            debug_assert!(
                old == EMPTY_SFT_NAME || new == EMPTY_SFT_NAME || old == new,
                "attempt to overwrite a non-empty chunk {} in SFT map (from {} to {})",
                chunk,
                old,
                new
            );
        }
        self_mut.sft[chunk] = sft;
    }

    pub fn is_in_any_space(&self, object: ObjectReference) -> bool {
        if object.to_address().chunk_index() >= self.sft.len() {
            return false;
        }
        self.get(object.to_address()).is_in_space(object)
    }

    #[cfg(feature = "is_mmtk_object")]
    pub fn is_mmtk_object(&self, addr: Address) -> bool {
        if addr.chunk_index() >= self.sft.len() {
            return false;
        }
        self.get(addr).is_mmtk_object(addr)
    }

    /// Make sure we have valid SFT entries for the object reference.
    #[cfg(debug_assertions)]
    pub fn assert_valid_entries_for_object<VM: VMBinding>(&self, object: ObjectReference) {
        let object_sft = self.get(object.to_address());
        let object_start_sft = self.get(VM::VMObjectModel::object_start_ref(object));

        debug_assert!(
            object_sft.name() != EMPTY_SFT_NAME,
            "Object {} has empty SFT",
            object
        );
        debug_assert_eq!(
            object_sft.name(),
            object_start_sft.name(),
            "Object {} has incorrect SFT entries (object start = {}, object = {}).",
            object,
            object_start_sft.name(),
            object_sft.name()
        );
    }
}

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

        // We need this lock: Othrewise, it is possible that one thread acquires pages in a new chunk, but not yet
        // set SFT for it (in grow_space()), and another thread acquires pages in the same chunk, which is not
        // a new chunk so grow_space() won't be called on it. The second thread could return a result in the chunk before
        // its SFT is properly set.
        let lock = self.common().acquire_lock.lock().unwrap();

        trace!("Reserving pages");
        let pr = self.get_page_resource();
        let pages_reserved = pr.reserve_pages(pages);
        trace!("Pages reserved");
        trace!("Polling ..");

        if should_poll && VM::VMActivePlan::global().poll(false, self.as_space()) {
            debug!("Collection required");
            assert!(allow_gc, "GC is not allowed here: collection is not initialized (did you call initialize_collection()?).");
            pr.clear_request(pages_reserved);
            drop(lock); // drop the lock before block
            VM::VMCollection::block_for_gc(VMMutatorThread(tls)); // We have checked that this is mutator
            unsafe { Address::zero() }
        } else {
            debug!("Collection not required");

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
                    // We thought we had memory to allocate, but somehow failed the allocation. Will force a GC.
                    assert!(
                        allow_gc,
                        "Physical allocation failed when GC is not allowed!"
                    );

                    let gc_performed = VM::VMActivePlan::global().poll(true, self.as_space());
                    debug_assert!(gc_performed, "GC not performed when forced.");
                    pr.clear_request(pages_reserved);
                    drop(lock); // drop the lock before block
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
