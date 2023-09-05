use atomic::Ordering;

use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::address::Address;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::metadata::mark_bit::MarkState;

use crate::util::{metadata, ObjectReference};

use crate::plan::{ObjectQueue, VectorObjectQueue};

use crate::policy::sft::GCWorkerMutRef;
use crate::vm::{ObjectModel, VMBinding};

/// This type implements a simple immortal collection
/// policy. Under this policy all that is required is for the
/// "collector" to propagate marks in a liveness trace.  It does not
/// actually collect.
pub struct ImmortalSpace<VM: VMBinding> {
    mark_state: MarkState,
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
    /// Is this used as VM space? If this is used as VM space, we never allocate into this space, but we trace objects normally.
    vm_space: bool,
}

impl<VM: VMBinding> SFT for ImmortalSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
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

impl<VM: VMBinding> Space<VM> for ImmortalSpace<VM> {
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
        self.common().initialize_sft(self.as_sft(), sft_map)
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }
}

use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for ImmortalSpace<VM> {
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

impl<VM: VMBinding> ImmortalSpace<VM> {
    pub fn new(args: crate::policy::space::PlanCreateSpaceArgs<VM>) -> Self {
        let vm_map = args.vm_map;
        let is_discontiguous = args.vmrequest.is_discontiguous();
        let common = CommonSpace::new(args.into_policy_args(
            false,
            true,
            metadata::extract_side_metadata(&[*VM::VMObjectModel::LOCAL_MARK_BIT_SPEC]),
        ));
        ImmortalSpace {
            mark_state: MarkState::new(),
            pr: if is_discontiguous {
                MonotonePageResource::new_discontiguous(vm_map)
            } else {
                MonotonePageResource::new_contiguous(common.start, common.extent, vm_map)
            },
            common,
            vm_space: false,
        }
    }

    #[cfg(feature = "vm_space")]
    pub fn new_vm_space(
        args: crate::policy::space::PlanCreateSpaceArgs<VM>,
        start: Address,
        size: usize,
    ) -> Self {
        assert!(!args.vmrequest.is_discontiguous());
        ImmortalSpace {
            mark_state: MarkState::new(),
            pr: MonotonePageResource::new_contiguous(start, size, args.vm_map),
            common: CommonSpace::new(args.into_policy_args(
                false,
                true,
                metadata::extract_side_metadata(&[*VM::VMObjectModel::LOCAL_MARK_BIT_SPEC]),
            )),
            vm_space: true,
        }
    }

    pub fn prepare(&mut self) {
        self.mark_state.on_global_prepare::<VM>();
        if self.vm_space {
            // If this is VM space, we never allocate into it, and we should reset the mark bit for the entire space.
            self.mark_state
                .on_block_reset::<VM>(self.common.start, self.common.extent)
        } else {
            // Otherwise, we reset the mark bit for the allocated regions.
            for (addr, size) in self.pr.iterate_allocated_regions() {
                debug!(
                    "{:?}: reset mark bit from {} to {}",
                    self.name(),
                    addr,
                    addr + size
                );
                self.mark_state.on_block_reset::<VM>(addr, size);
            }
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
        if self.mark_state.test_and_mark::<VM>(object) {
            queue.enqueue(object);
        }
        object
    }
}
