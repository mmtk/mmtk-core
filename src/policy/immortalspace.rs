use atomic::Ordering;

use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::address::Address;
use crate::util::heap::{MonotonePageResource, PageResource, VMRequest};

use crate::util::{metadata, ObjectReference};

use crate::plan::{ObjectQueue, VectorObjectQueue};

use crate::plan::PlanConstraints;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::space::SpaceOptions;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::vm::{ObjectModel, VMBinding};

/// This type implements a simple immortal collection
/// policy. Under this policy all that is required is for the
/// "collector" to propagate marks in a liveness trace.  It does not
/// actually collect.
pub struct ImmortalSpace<VM: VMBinding> {
    mark_state: u8,
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
}

const GC_MARK_BIT_MASK: u8 = 1;

impl<VM: VMBinding> SFT for ImmortalSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, _object: ObjectReference) -> bool {
        true
    }
    #[inline(always)]
    fn is_reachable(&self, object: ObjectReference) -> bool {
        let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        );
        old_value == self.mark_state
    }
    fn pin_object(&self, _object: ObjectReference) -> bool {
        false
    }
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        false
    }
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
    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        );
        let new_value = (old_value & GC_MARK_BIT_MASK) | self.mark_state;
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(
            object,
            new_value,
            None,
            Ordering::SeqCst,
        );

        if self.common.needs_log_bit {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(object, Ordering::SeqCst);
        }
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::set_alloc_bit(object);
    }
    #[inline(always)]
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

    fn initialize_sft(&self) {
        self.common().initialize_sft(self.as_sft())
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }
}

use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for ImmortalSpace<VM> {
    #[inline(always)]
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.trace_object(queue, object)
    }
    #[inline(always)]
    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        false
    }
}

impl<VM: VMBinding> ImmortalSpace<VM> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
        constraints: &'static PlanConstraints,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: true,
                needs_log_bit: constraints.needs_log_bit,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: metadata::extract_side_metadata(&[
                        *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                    ]),
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        ImmortalSpace {
            mark_state: 0,
            pr: if vmrequest.is_discontiguous() {
                MonotonePageResource::new_discontiguous(vm_map)
            } else {
                MonotonePageResource::new_contiguous(common.start, common.extent, vm_map)
            },
            common,
        }
    }

    fn test_and_mark(object: ObjectReference, value: u8) -> bool {
        loop {
            let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
                object,
                None,
                Ordering::SeqCst,
            );
            if old_value == value {
                return false;
            }

            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    old_value ^ GC_MARK_BIT_MASK,
                    None,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break;
            }
        }
        true
    }

    pub fn prepare(&mut self) {
        self.mark_state = GC_MARK_BIT_MASK - self.mark_state;
    }

    pub fn release(&mut self) {}

    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        #[cfg(feature = "global_alloc_bit")]
        debug_assert!(
            crate::util::alloc_bit::is_alloced(object),
            "{:x}: alloc bit not set",
            object
        );
        if ImmortalSpace::<VM>::test_and_mark(object, self.mark_state) {
            queue.enqueue(object);
        }
        object
    }
}
