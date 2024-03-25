use atomic::Atomic;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::address::Address;

use crate::util::conversions;
use crate::util::heap::gc_trigger::GCTrigger;
use crate::util::heap::layout::vm_layout::vm_layout;
use crate::util::heap::PageResource;
use crate::util::heap::VMRequest;
use crate::util::memory::MmapStrategy;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::opaque_pointer::*;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

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
    cursor: Atomic<Address>,
    /// Heap range end
    limit: Address,
    /// start of this space
    start: Address,
    /// Total bytes for the space
    total_bytes: usize,
    /// Zero memory after slow-path allocation
    slow_path_zeroing: bool,
    metadata: SideMetadataContext,
    gc_trigger: Arc<GCTrigger<VM>>,
}

impl<VM: VMBinding> SFT for LockFreeImmortalSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, _object: ObjectReference) -> bool {
        unimplemented!()
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
        true
    }
    fn is_movable(&self) -> bool {
        unimplemented!()
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        unimplemented!()
    }
    fn initialize_object_metadata(&self, _object: ObjectReference, _alloc: bool) {
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::set_vo_bit::<VM>(_object);
    }
    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        crate::util::metadata::vo_bit::is_vo_bit_set_for_addr::<VM>(addr).is_some()
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

    fn get_gc_trigger(&self) -> &GCTrigger<VM> {
        &self.gc_trigger
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }

    fn initialize_sft(&self, sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        unsafe { sft_map.eager_initialize(self.as_sft(), self.start, self.total_bytes) };
    }

    fn reserved_pages(&self) -> usize {
        let cursor = self.cursor.load(Ordering::Relaxed);
        let data_pages = conversions::bytes_to_pages_up(self.limit - cursor);
        let meta_pages = self.metadata.calculate_reserved_pages(data_pages);
        data_pages + meta_pages
    }

    fn acquire(&self, _tls: VMThread, pages: usize) -> Address {
        trace!("LockFreeImmortalSpace::acquire");
        let bytes = conversions::pages_to_bytes(pages);
        let start = self
            .cursor
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |addr| {
                Some(addr.add(bytes))
            })
            .expect("update cursor failed");
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
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        _queue: &mut Q,
        _object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        unreachable!()
    }
    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        unreachable!()
    }
}

impl<VM: VMBinding> LockFreeImmortalSpace<VM> {
    #[allow(dead_code)] // Only used with certain features.
    pub fn new(args: crate::policy::space::PlanCreateSpaceArgs<VM>) -> Self {
        let slow_path_zeroing = args.zeroed;

        // Get the total bytes for the heap.
        let total_bytes = match *args.options.gc_trigger {
            crate::util::options::GCTriggerSelector::FixedHeapSize(bytes) => bytes,
            _ => unimplemented!(),
        };
        assert!(
            total_bytes <= vm_layout().available_bytes(),
            "Initial requested memory ({} bytes) overflows the heap. Max heap size is {} bytes.",
            total_bytes,
            vm_layout().available_bytes()
        );
        // Align up to chunks
        let aligned_total_bytes = crate::util::conversions::raw_align_up(
            total_bytes,
            crate::util::heap::vm_layout::BYTES_IN_CHUNK,
        );

        // Create a VM request of fixed size
        let vmrequest = VMRequest::fixed_size(aligned_total_bytes);
        // Reserve the space
        let VMRequest::Extent { extent, top } = vmrequest else {
            unreachable!()
        };
        let start = args.heap.reserve(extent, top);

        let space = Self {
            name: args.name,
            cursor: Atomic::new(start),
            limit: start + aligned_total_bytes,
            start,
            total_bytes: aligned_total_bytes,
            slow_path_zeroing,
            metadata: SideMetadataContext {
                global: args.global_side_metadata_specs,
                local: vec![],
            },
            gc_trigger: args.gc_trigger,
        };

        // Eagerly memory map the entire heap (also zero all the memory)
        let strategy = if *args.options.transparent_hugepages {
            MmapStrategy::TransparentHugePages
        } else {
            MmapStrategy::Normal
        };
        crate::util::memory::dzmmap_noreplace(start, aligned_total_bytes, strategy).unwrap();
        if space
            .metadata
            .try_map_metadata_space(start, aligned_total_bytes)
            .is_err()
        {
            // TODO(Javad): handle meta space allocation failure
            panic!("failed to mmap meta memory");
        }

        space
    }
}
