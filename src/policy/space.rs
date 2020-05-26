use crate::util::conversions::*;
use crate::util::Address;
use crate::util::ObjectReference;

use crate::util::heap::layout::vm_layout_constants::{AVAILABLE_BYTES, LOG_BYTES_IN_CHUNK};
use crate::util::heap::layout::vm_layout_constants::{AVAILABLE_END, AVAILABLE_START};
use crate::util::heap::{PageResource, VMRequest};
use crate::vm::{ActivePlan, Collection, ObjectModel};

use crate::plan::Plan;

use crate::util::constants::LOG_BYTES_IN_MBYTE;
use crate::util::conversions;
use crate::util::OpaquePointer;

use crate::mmtk::SFT_MAP;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::layout::vm_layout_constants::MAX_CHUNKS;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::heap::HeapMeta;

use crate::vm::VMBinding;
use std::marker::PhantomData;

/**
 * Space Function Table (SFT).
 *
 * This trait captures functions that reflect _space-specific per-object
 * semantics_.   These functions are implemented for each object via a special
 * space-based dynamic dispatch mechanism where the semantics are _not_
 * determined by the object's _type_, but rather, are determined by the _space_
 * that the object is in.
 *
 * The underlying mechanism exploits the fact that spaces use the address space
 * at an MMTk chunk granularity with the consequence that each chunk maps to
 * exactluy one space, so knowing the chunk for an object reveals its space.
 * The dispatch then works by performing simple address arithmetic on the object
 * reference to find a chunk index which is used to index a table which returns
 * the space.   The relevant function is then dispatched against that space
 * object.
 *
 * We use the SFT trait to simplify typing for Rust, so our table is a
 * table of SFT rather than Space.
 */
pub trait SFT {
    fn is_live(&self, object: ObjectReference) -> bool;
    fn is_movable(&self) -> bool;
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool;
    fn initialize_header(&self, object: ObjectReference, alloc: bool);
}

unsafe impl Sync for SFTMap {}

struct EmptySpaceSFT {}
unsafe impl Sync for EmptySpaceSFT {}
impl SFT for EmptySpaceSFT {
    fn is_live(&self, object: ObjectReference) -> bool {
        panic!(
            "Called is_live() on {:x}, which maps to an empty space",
            object
        )
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
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

    fn initialize_header(&self, object: ObjectReference, _alloc: bool) {
        panic!(
            "Called initialize_header() on {:x}, which maps to an empty space",
            object
        )
    }
}

#[derive(Default)]
pub struct SFTMap {
    sft: Vec<*const (dyn SFT + Sync)>,
}

impl SFTMap {
    pub fn new() -> Self {
        SFTMap {
            sft: vec![&EmptySpaceSFT {}; MAX_CHUNKS],
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

    pub fn get(&self, address: Address) -> &'static dyn SFT {
        unsafe { &*self.sft[address.chunk_index()] }
    }

    pub fn update(&self, space: *const (dyn SFT + Sync), start: Address, chunks: usize) {
        let first = start.chunk_index();
        for chunk in first..(first + chunks) {
            self.set(chunk, space);
        }
    }

    pub fn clear(&self, chunk_idx: usize) {
        self.set(chunk_idx, &EmptySpaceSFT {});
    }

    fn set(&self, chunk: usize, sft: *const (dyn SFT + Sync)) {
        /*
         * This is safe (only) because a) this is only called during the
         * allocation and deallocation of chunks, which happens under a global
         * lock, and b) it only transitions from empty to valid and valid to
         * empty, so if there were a race to view the contents, in the one case
         * it would either see the new (valid) space or an empty space (both of
         * which are reasonable), and in the other case it would either see the
         * old (valid) space or an empty space, both of which are valid.
         */
        let self_mut: &mut Self = unsafe { self.mut_self() };
        self_mut.sft[chunk] = sft;
    }
}

pub trait Space<VM: VMBinding>: Sized + 'static + SFT + Sync {
    type PR: PageResource<VM, Space = Self>;

    fn init(&mut self, vm_map: &'static VMMap);

    fn acquire(&self, tls: OpaquePointer, pages: usize) -> Address {
        trace!("Space.acquire, tls={:?}", tls);
        // debug_assert!(tls != 0);
        let allow_poll = unsafe { VM::VMActivePlan::is_mutator(tls) }
            && VM::VMActivePlan::global().is_initialized();

        trace!("Reserving pages");
        let pr = self.common().pr.as_ref().unwrap();
        let pages_reserved = pr.reserve_pages(pages);
        trace!("Pages reserved");

        // FIXME: Possibly unnecessary borrow-checker fighting
        let me = unsafe { &*(self as *const Self) };

        trace!("Polling ..");

        if allow_poll && VM::VMActivePlan::global().poll::<Self::PR>(false, me) {
            trace!("Collection required");
            pr.clear_request(pages_reserved);
            VM::VMCollection::block_for_gc(tls);
            unsafe { Address::zero() }
        } else {
            trace!("Collection not required");
            let rtn = pr.get_new_pages(pages_reserved, pages, self.common().zeroed, tls);
            if rtn.is_zero() {
                if !allow_poll {
                    panic!("Physical allocation failed when polling not allowed!");
                }

                let gc_performed = VM::VMActivePlan::global().poll::<Self::PR>(true, me);
                debug_assert!(gc_performed, "GC not performed when forced.");
                pr.clear_request(pages_reserved);
                VM::VMCollection::block_for_gc(tls);
                unsafe { Address::zero() }
            } else {
                rtn
            }
        }
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        let start = VM::VMObjectModel::ref_to_address(object);
        if !self.common().descriptor.is_contiguous() {
            self.common().vm_map().get_descriptor_for_address(start) == self.common().descriptor
        } else {
            start >= self.common().start && start < self.common().start + self.common().extent
        }
    }

    // UNSAFE: potential data race as this mutates 'common'
    unsafe fn grow_discontiguous_space(&self, chunks: usize) -> Address {
        // FIXME
        let new_head: Address = self.common().vm_map().allocate_contiguous_chunks(
            self.common().descriptor,
            chunks,
            self.common().head_discontiguous_region,
        );
        if new_head.is_zero() {
            return Address::zero();
        }

        self.unsafe_common_mut().head_discontiguous_region = new_head;
        new_head
    }

    /**
     * This hook is called by page resources each time a space grows.  The space may
     * tap into the hook to monitor heap growth.  The call is made from within the
     * page resources' critical region, immediately before yielding the lock.
     *
     * @param start The start of the newly allocated space
     * @param bytes The size of the newly allocated space
     * @param new_chunk {@code true} if the new space encroached upon or started a new chunk or chunks.
     */
    fn grow_space(&self, start: Address, bytes: usize, new_chunk: bool) {
        if new_chunk {
            let chunks = conversions::bytes_to_chunks_up(bytes);
            SFT_MAP.update(self as *const (dyn SFT + Sync), start, chunks);
        }
    }

    /**
     *  Ensure this space is marked as mapped -- used when the space is already
     *  mapped (e.g. for a vm image which is externally mmapped.)
     */
    fn ensure_mapped(&self) {
        let chunks = conversions::bytes_to_chunks_up(self.common().extent);
        SFT_MAP.update(self as *const (dyn SFT + Sync), self.common().start, chunks);
        use crate::util::heap::layout::mmapper::Mmapper;
        self.common()
            .mmapper
            .mark_as_mapped(self.common().start, self.common().extent);
    }

    fn reserved_pages(&self) -> usize {
        self.common().pr.as_ref().unwrap().reserved_pages()
    }

    fn get_name(&self) -> &'static str {
        self.common().name
    }

    fn common(&self) -> &CommonSpace<VM, Self::PR>;
    fn common_mut(&mut self) -> &mut CommonSpace<VM, Self::PR> {
        // SAFE: Reference is exclusive
        unsafe { self.unsafe_common_mut() }
    }

    // UNSAFE: This get's a mutable reference from self
    // (i.e. make sure their are no concurrent accesses through self when calling this)_
    #[allow(clippy::mut_from_ref)]
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM, Self::PR>;

    fn release_discontiguous_chunks(&mut self, chunk: Address) {
        debug_assert!(chunk == conversions::chunk_align_down(chunk));
        if chunk == self.common().head_discontiguous_region {
            self.common_mut().head_discontiguous_region =
                self.common().vm_map().get_next_contiguous_region(chunk);
        }
        self.common().vm_map().free_contiguous_chunks(chunk);
    }

    fn release_multiple_pages(&mut self, start: Address);

    unsafe fn release_all_chunks(&self) {
        self.common()
            .vm_map()
            .free_all_chunks(self.common().head_discontiguous_region);
        self.unsafe_common_mut().head_discontiguous_region = Address::zero();
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
                VMRequest::RequestExtent { extent, .. } => {
                    print!(" E {}", extent);
                }
                VMRequest::RequestFraction { frac, .. } => {
                    print!(" F {}", frac);
                }
                _ => {}
            }
        } else {
            let mut a = common.head_discontiguous_region;
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
}

pub struct CommonSpace<VM: VMBinding, PR: PageResource<VM>> {
    pub name: &'static str,
    pub descriptor: SpaceDescriptor,
    pub vmrequest: VMRequest,

    immortal: bool,
    movable: bool,
    pub contiguous: bool,
    pub zeroed: bool,

    pub pr: Option<PR>,
    pub start: Address,
    pub extent: usize,
    pub head_discontiguous_region: Address,

    pub vm_map: &'static VMMap,
    pub mmapper: &'static Mmapper,

    p: PhantomData<VM>,
}

pub struct SpaceOptions {
    pub name: &'static str,
    pub movable: bool,
    pub immortal: bool,
    pub zeroed: bool,
    pub vmrequest: VMRequest,
}

const DEBUG: bool = false;

unsafe impl<VM: VMBinding, PR: PageResource<VM>> Sync for CommonSpace<VM, PR> {}

impl<VM: VMBinding, PR: PageResource<VM>> CommonSpace<VM, PR> {
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
            immortal: opt.immortal,
            movable: opt.movable,
            contiguous: true,
            zeroed: opt.zeroed,
            pr: None,
            start: unsafe { Address::zero() },
            extent: 0,
            head_discontiguous_region: unsafe { Address::zero() },
            vm_map,
            mmapper,
            p: PhantomData,
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
            VMRequest::RequestFraction { frac, top: _top } => (get_frac_available(frac), _top),
            VMRequest::RequestExtent {
                extent: _extent,
                top: _top,
            } => (_extent, _top),
            VMRequest::RequestFixed {
                extent: _extent,
                top: _top,
                ..
            } => (_extent, _top),
            _ => unreachable!(),
        };

        if extent != raw_align_up(extent, BYTES_IN_CHUNK) {
            panic!(
                "{} requested non-aligned extent: {} bytes",
                rtn.name, extent
            );
        }

        let start: Address;
        if let VMRequest::RequestFixed { start: _start, .. } = vmrequest {
            start = _start;
            if start != chunk_align_up(start) {
                panic!("{} starting on non-aligned boundary: {}", rtn.name, start);
            }
        } else {
            // FIXME
            //if (HeapLayout.vmMap.isFinalized()) VM.assertions.fail("heap is narrowed after regionMap is finalized: " + name);
            start = heap.reserve(extent, top);
        }

        rtn.contiguous = true;
        rtn.start = start;
        rtn.extent = extent;
        // FIXME
        rtn.descriptor = SpaceDescriptor::create_descriptor_from_heap_range(start, start + extent);
        // VM.memory.setHeapRange(index, start, start.plus(extent));
        vm_map.insert(start, extent, rtn.descriptor);

        if DEBUG {
            println!("{} {} {} {}", rtn.name, start, start + extent, extent);
        }

        rtn
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
