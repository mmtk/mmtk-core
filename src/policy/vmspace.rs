use atomic::Ordering;

use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::address::Address;
use crate::util::heap::{MonotonePageResource, PageResource};

use crate::util::{metadata, ObjectReference};

use crate::plan::{ObjectQueue, VectorObjectQueue};

use crate::policy::sft::GCWorkerMutRef;
use crate::vm::{ObjectModel, VMBinding};

/// This type implements VM space, a space managed by the runtime. The space helps us trace and inspect objects
/// in the same way as other MMTk spaces, rather than treating them as special cases.

// We used to use ImmortalSpace as vm space up to commit 43e8a92b507ce9b8f771f31d2dbef7eee93f3cc2, and only
// JikesRVM was using VM space at that point. Java MMTk does the same thing for JikesRVM (using immortal space as boot space),
// and our ImmortalSpace in 43e8a92b507ce9b8f771f31d2dbef7eee93f3cc2 was implemented exactly the same as the immortal space in JikesRVM's Java MMTk.
// However, we introduce changes after 43e8a92b507ce9b8f771f31d2dbef7eee93f3cc2, and ImmortalSpace starts to use MarkState.
// MarkState provides an abstraction of how we flip mark bit, reset mark bit, and check mark bit, depending on the location
// of the mark bit (on side or in header). In that case, our ImmortalSpace is no longer the same as Java
// MMTk. As JikesRVM has assumptions in its boot image generation about the space, the new immortal space can no longer be
// used as vm space for JikesRVM. To temperarily work around the issue, we duplicate ImmortalSpace from commit 43e8a92b507ce9b8f771f31d2dbef7eee93f3cc2
// into this VMSpace, so the change to ImmortalSpace does not break JikesRVM's vm space.
// TODO: We will provide a new implementation of this VMSpace to accomodate JikesRVM, and other VMs.
pub struct VMSpace<VM: VMBinding> {
    mark_state: u8,
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
}

const GC_MARK_BIT_MASK: u8 = 1;

impl<VM: VMBinding> SFT for VMSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, _object: ObjectReference) -> bool {
        true
    }
    fn is_reachable(&self, object: ObjectReference) -> bool {
        let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        );
        old_value == self.mark_state
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

    fn initialize_sft(&self) {
        self.common().initialize_sft(self.as_sft())
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("VMSpace only releases pages enmasse")
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
        let vm_map = args.vm_map;
        let is_discontiguous = args.vmrequest.is_discontiguous();
        let common = CommonSpace::new(args.into_policy_args(
            false,
            true,
            metadata::extract_side_metadata(&[*VM::VMObjectModel::LOCAL_MARK_BIT_SPEC]),
        ));
        VMSpace {
            mark_state: 0,
            pr: if is_discontiguous {
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
        #[cfg(feature = "vo_bit")]
        debug_assert!(
            crate::util::metadata::vo_bit::is_vo_bit_set::<VM>(object),
            "{:x}: VO bit not set",
            object
        );
        if VMSpace::<VM>::test_and_mark(object, self.mark_state) {
            queue.enqueue(object);
        }
        object
    }
}
