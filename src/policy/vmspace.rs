use crate::mmtk::SFT_MAP;
use crate::plan::{ObjectQueue, VectorObjectQueue};
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::address::Address;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::heap::externalpageresource::{ExternalPageResource, ExternalPages};
use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
use crate::util::heap::PageResource;
use crate::util::metadata::mark_bit::MarkState;
use crate::util::opaque_pointer::*;
use crate::util::ObjectReference;
use crate::vm::{ObjectModel, VMBinding};

use std::sync::atomic::Ordering;

/// A special space for VM/Runtime managed memory. The implementation is similar to [`crate::policy::immortalspace::ImmortalSpace`],
/// except that VM space does not allocate. Instead, the runtime can add regions that are externally managed
/// and mmapped to the space, and allow objects in those regions to be traced in the same way
/// as other MMTk objects allocated by MMTk.
pub struct VMSpace<VM: VMBinding> {
    mark_state: MarkState,
    common: CommonSpace<VM>,
    pr: ExternalPageResource<VM>,
}

impl<VM: VMBinding> SFT for VMSpace<VM> {
    fn name(&self) -> &str {
        self.common.name
    }
    fn is_live(&self, _object: ObjectReference) -> bool {
        true
    }
    fn is_reachable(&self, object: ObjectReference) -> bool {
        self.mark_state.is_marked::<VM>(object)
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
        false
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        self.mark_state
            .on_object_metadata_initialization::<VM>(object);
        if self.common.needs_log_bit {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(object, Ordering::SeqCst);
        }
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::set_vo_bit::<VM>(object);
    }
    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        crate::util::metadata::vo_bit::is_vo_bit_set_for_addr::<VM>(addr).is_some()
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

impl<VM: VMBinding> Space<VM> for VMSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }
    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn initialize_sft(&self, sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        // Initialize sft for current external pages. This method is called at the end of plan creation.
        // So we only set SFT for VM regions that are set by options (we skipped sft initialization for them earlier).
        let vm_regions = self.pr.get_external_pages();
        // We should have at most one region at this point (set by the option). If we allow setting multiple VM spaces through options,
        // we can remove this assertion.
        assert!(vm_regions.len() <= 1);
        for external_pages in vm_regions.iter() {
            // Chunk align things.
            let start = external_pages.start.align_down(BYTES_IN_CHUNK);
            let size = external_pages.end.align_up(BYTES_IN_CHUNK) - start;
            // The region should be empty in SFT map -- if they were set before this point, there could be invalid SFT pointers.
            debug_assert_eq!(
                sft_map.get_checked(start).name(),
                crate::policy::sft::EMPTY_SFT_NAME
            );
            // Set SFT
            assert!(sft_map.has_sft_entry(start), "The VM space start (aligned to {}) does not have a valid SFT entry. Possibly the address range is not in the address range we use.", start);
            unsafe {
                sft_map.eager_initialize(self.as_sft(), start, size);
            }
        }
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        unreachable!()
    }

    fn acquire(&self, _tls: VMThread, _pages: usize) -> Address {
        unreachable!()
    }

    fn address_in_space(&self, start: Address) -> bool {
        // The default implementation checks with vm map. But vm map has some assumptions about
        // the address range for spaces and the VM space may break those assumptions (as the space is
        // mmapped by the runtime rather than us). So we we use SFT here.

        // However, SFT map may not be an ideal solution either for 64 bits. The default
        // implementation of SFT map on 64 bits is `SFTSpaceMap`, which maps the entire address
        // space into an index between 0 and 31, and assumes any address with the same index
        // is in the same space (with the same SFT). MMTk spaces uses 1-16. We guarantee that
        // VM space does not overlap with the address range that MMTk spaces may use. So
        // any region used as VM space will have an index of 0, or 17-31, and all the addresses
        // that are mapped to the same index will be considered as in the VM space. That means,
        // after we map a region as VM space, the nearby addresses will also be considered
        // as in the VM space if we use the default `SFTSpaceMap`. We can guarantee the nearby
        // addresses are not MMTk spaces, but we cannot tell whether they really in the VM space
        // or not.
        // A solution to this is to use `SFTDenseChunkMap` if `vm_space` is enabled on 64 bits.
        // `SFTDenseChunkMap` has an overhead of a few percentages (~3%) compared to `SFTSpaceMap`.
        SFT_MAP.get_checked(start).name() == self.name()
    }
}

use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for VMSpace<VM> {
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

impl<VM: VMBinding> VMSpace<VM> {
    pub fn new(args: crate::policy::space::PlanCreateSpaceArgs<VM>) -> Self {
        let (vm_space_start, vm_space_size) =
            (*args.options.vm_space_start, *args.options.vm_space_size);
        let space = Self {
            mark_state: MarkState::new(),
            pr: ExternalPageResource::new(args.vm_map),
            common: CommonSpace::new(args.into_policy_args(
                false,
                true,
                crate::util::metadata::extract_side_metadata(&[
                    *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                ]),
            )),
        };

        if !vm_space_start.is_zero() {
            // Do not set sft here, as the space may be moved. We do so for those regions in `initialize_sft`.
            space.set_vm_region_inner(vm_space_start, vm_space_size, false);
        }

        space
    }

    pub fn set_vm_region(&mut self, start: Address, size: usize) {
        self.set_vm_region_inner(start, size, true);
    }

    fn set_vm_region_inner(&self, start: Address, size: usize, set_sft: bool) {
        assert!(size > 0);
        assert!(!start.is_zero());

        let end = start + size;

        let chunk_start = start.align_down(BYTES_IN_CHUNK);
        let chunk_end = end.align_up(BYTES_IN_CHUNK);
        let chunk_size = chunk_end - chunk_start;

        // For simplicity, VMSpace has to be outside our available heap range.
        // TODO: Allow VMSpace in our available heap range.
        assert!(Address::range_intersection(
            &(chunk_start..chunk_end),
            &crate::util::heap::layout::available_range()
        )
        .is_empty());

        debug!(
            "Align VM space ({}, {}) to chunk ({}, {})",
            start, end, chunk_start, chunk_end
        );

        // Mark as mapped in mmapper
        self.common.mmapper.mark_as_mapped(chunk_start, chunk_size);
        // Map side metadata
        self.common
            .metadata
            .try_map_metadata_space(chunk_start, chunk_size)
            .unwrap();
        // Insert to vm map: it would be good if we can make VM map aware of the region. However, the region may be outside what we can map in our VM map implementation.
        // self.common.vm_map.insert(chunk_start, chunk_size, self.common.descriptor);
        // Set SFT if we should
        if set_sft {
            assert!(SFT_MAP.has_sft_entry(chunk_start), "The VM space start (aligned to {}) does not have a valid SFT entry. Possibly the address range is not in the address range we use.", chunk_start);
            unsafe {
                SFT_MAP.update(self.as_sft(), chunk_start, chunk_size);
            }
        }

        self.pr.add_new_external_pages(ExternalPages {
            start: start.align_down(BYTES_IN_PAGE),
            end: end.align_up(BYTES_IN_PAGE),
        });
    }

    pub fn prepare(&mut self) {
        self.mark_state.on_global_prepare::<VM>();
        for external_pages in self.pr.get_external_pages().iter() {
            self.mark_state.on_block_reset::<VM>(
                external_pages.start,
                external_pages.end - external_pages.start,
            );
        }
    }

    pub fn release(&mut self) {
        self.mark_state.on_global_release::<VM>();
    }

    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        #[cfg(feature = "vo_bit")]
        debug_assert!(
            crate::util::metadata::vo_bit::is_vo_bit_set::<VM>(object),
            "{:x}: VO bit not set",
            object
        );
        debug_assert!(self.in_space(object));
        if self.mark_state.test_and_mark::<VM>(object) {
            queue.enqueue(object);
        }
        object
    }
}
