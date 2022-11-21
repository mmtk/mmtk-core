use crate::mmtk::SFT_MAP;
use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::address::Address;
use crate::util::heap::PageResource;
use crate::util::ObjectReference;

use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft_map::SFTMap;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::{AVAILABLE_BYTES, AVAILABLE_START};
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::opaque_pointer::*;
use crate::util::options::Options;
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

/// This type implements a lock free version of the immortal collection
/// policy. This is close to the OpenJDK's epsilon GC.
/// Different from the normal ImmortalSpace, this version should only
/// be used by NoGC plan, and it now uses the whole heap range.
// FIXME: It is wrong that the space uses the whole heap range. It has to reserve its own
// range from HeapMeta, and not clash with other spaces.
pub struct LockFreeImmortalSpace<VM: VMBinding> {
    #[allow(unused)]
    name: &'static str,
    /// Heap range start
    ///
    /// We use `AtomicUsize` instead of `Address` here to atomically bumping this cursor.
    /// TODO: Better address type here (Atomic<Address>?)
    cursor: AtomicUsize,
    /// Heap range end
    limit: Address,
    /// start of this space
    start: Address,
    /// Total bytes for the space
    extent: usize,
    /// Zero memory after slow-path allocation
    slow_path_zeroing: bool,
    metadata: SideMetadataContext,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> SFT for LockFreeImmortalSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, _object: ObjectReference) -> bool {
        unimplemented!()
    }
    fn pin_object(&self, _object: ObjectReference) -> bool {
        unimplemented!()
    }
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        unimplemented!()
    }
    fn is_object_pinned(&self, _object: ObjectReference) -> bool {
        unimplemented!()
    }
    fn is_movable(&self) -> bool {
        unimplemented!()
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        unimplemented!()
    }
    fn initialize_object_metadata(&self, _object: ObjectReference, _alloc: bool) {
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::set_alloc_bit(_object);
    }
    fn sft_trace_object(
        &self,
        _queue: &mut VectorObjectQueue,
        _object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        unreachable!()
    }
}

impl<VM: VMBinding> Space<VM> for LockFreeImmortalSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        unimplemented!()
    }
    fn common(&self) -> &CommonSpace<VM> {
        unimplemented!()
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }

    fn initialize_sft(&self) {
        unsafe { SFT_MAP.update(self.as_sft(), self.start, self.extent) };
    }

    fn reserved_pages(&self) -> usize {
        let cursor = unsafe { Address::from_usize(self.cursor.load(Ordering::Relaxed)) };
        let data_pages = conversions::bytes_to_pages_up(self.limit - cursor);
        let meta_pages = self.metadata.calculate_reserved_pages(data_pages);
        data_pages + meta_pages
    }

    fn acquire(&self, _tls: VMThread, pages: usize) -> Address {
        let bytes = conversions::pages_to_bytes(pages);
        let start = unsafe { Address::from_usize(self.cursor.fetch_add(bytes, Ordering::Relaxed)) };
        if start + bytes > self.limit {
            panic!("OutOfMemory")
        }
        if self.slow_path_zeroing {
            crate::util::memory::zero(start, bytes);
        }
        start
    }

    /// Get the name of the space
    ///
    /// We have to override the default implementation because
    /// LockFreeImmortalSpace doesn't have a common space
    fn get_name(&self) -> &'static str {
        "LockFreeImmortalSpace"
    }

    /// We have to override the default implementation because
    /// LockFreeImmortalSpace doesn't put metadata in a common space
    fn verify_side_metadata_sanity(&self, side_metadata_sanity_checker: &mut SideMetadataSanity) {
        side_metadata_sanity_checker
            .verify_metadata_context(std::any::type_name::<Self>(), &self.metadata)
    }
}

use crate::plan::{ObjectQueue, VectorObjectQueue};
use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for LockFreeImmortalSpace<VM> {
    #[inline(always)]
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        _queue: &mut Q,
        _object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        unreachable!()
    }
    #[inline(always)]
    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        unreachable!()
    }
}

impl<VM: VMBinding> LockFreeImmortalSpace<VM> {
    #[allow(dead_code)] // Only used with certain features.
    pub fn new(
        name: &'static str,
        slow_path_zeroing: bool,
        options: &Options,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
    ) -> Self {
        let total_bytes = *options.heap_size;
        assert!(
            total_bytes <= AVAILABLE_BYTES,
            "Initial requested memory ({} bytes) overflows the heap. Max heap size is {} bytes.",
            total_bytes,
            AVAILABLE_BYTES
        );

        // FIXME: This space assumes that it can use the entire heap range, which is definitely wrong.
        // https://github.com/mmtk/mmtk-core/issues/314
        let space = Self {
            name,
            cursor: AtomicUsize::new(AVAILABLE_START.as_usize()),
            limit: AVAILABLE_START + total_bytes,
            start: AVAILABLE_START,
            extent: total_bytes,
            slow_path_zeroing,
            metadata: SideMetadataContext {
                global: global_side_metadata_specs,
                local: vec![],
            },
            phantom: PhantomData,
        };

        // Eagerly memory map the entire heap (also zero all the memory)
        crate::util::memory::dzmmap_noreplace(AVAILABLE_START, total_bytes).unwrap();
        if space
            .metadata
            .try_map_metadata_space(AVAILABLE_START, total_bytes)
            .is_err()
        {
            // TODO(Javad): handle meta space allocation failure
            panic!("failed to mmap meta memory");
        }

        space
    }
}
