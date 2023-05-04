use crate::plan::{CreateGeneralPlanArgs, CreateSpecificPlanArgs};
use crate::plan::{ObjectQueue, VectorObjectQueue};
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::address::Address;
use crate::util::heap::HeapMeta;
use crate::util::heap::PageResource;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

pub struct VMSpace<VM: VMBinding> {
    inner: Option<ImmortalSpace<VM>>,
    // Save it
    args: CreateSpecificPlanArgs<VM>,
}

impl<VM: VMBinding> SFT for VMSpace<VM> {
    fn name(&self) -> &str {
        self.space().name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        self.space().is_live(object)
    }
    fn is_reachable(&self, object: ObjectReference) -> bool {
        self.space().is_reachable(object)
    }
    #[cfg(feature = "object_pinning")]
    fn pin_object(&self, object: ObjectReference) -> bool {
        self.space().pin_object(object)
    }
    #[cfg(feature = "object_pinning")]
    fn unpin_object(&self, object: ObjectReference) -> bool {
        self.space().unpin_object(object)
    }
    #[cfg(feature = "object_pinning")]
    fn is_object_pinned(&self, object: ObjectReference) -> bool {
        self.space().is_object_pinned(object)
    }
    fn is_movable(&self) -> bool {
        self.space().is_movable()
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        self.space().is_sane()
    }
    fn initialize_object_metadata(&self, _object: ObjectReference, _alloc: bool) {
        // TODO: Do we expect runtime to initialize object metadata?
        todo!()
    }
    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        self.space().is_mmtk_object(addr)
    }
    fn sft_trace_object(
        &self,
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        worker: GCWorkerMutRef,
    ) -> ObjectReference {
        self.space().sft_trace_object(queue, object, worker)
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
        self.space().get_page_resource()
    }
    fn common(&self) -> &CommonSpace<VM> {
        self.space().common()
    }

    fn initialize_sft(&self) {
        if self.inner.is_some() {
            self.common().initialize_sft(self.as_sft())
        }
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }

    fn verify_side_metadata_sanity(&self, side_metadata_sanity_checker: &mut SideMetadataSanity) {
        side_metadata_sanity_checker.verify_metadata_context(
            std::any::type_name::<Self>(),
            &SideMetadataContext {
                global: self.args.global_side_metadata_specs.clone(),
                local: vec![],
            },
        )
    }

    fn address_in_space(&self, start: Address) -> bool {
        if let Some(space) = self.space_maybe() {
            space.address_in_space(start)
        } else {
            false
        }
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
    pub fn new(args: &mut CreateSpecificPlanArgs<VM>) -> Self {
        let args_clone = CreateSpecificPlanArgs {
            global_args: CreateGeneralPlanArgs {
                vm_map: args.global_args.vm_map,
                mmapper: args.global_args.mmapper,
                heap: HeapMeta::new(), // we do not use this
                options: args.global_args.options.clone(),
                gc_trigger: args.global_args.gc_trigger.clone(),
                scheduler: args.global_args.scheduler.clone(),
            },
            constraints: args.constraints,
            global_side_metadata_specs: args.global_side_metadata_specs.clone(),
        };
        if !args.global_args.options.vm_space_start.is_zero() {
            Self {
                inner: Some(Self::create_space(args, None)),
                args: args_clone,
            }
        } else {
            Self {
                inner: None,
                args: args_clone,
            }
        }
    }

    pub fn lazy_initialize(&mut self, start: Address, size: usize) {
        assert!(self.inner.is_none(), "VM space has been initialized");
        self.inner = Some(Self::create_space(&mut self.args, Some((start, size))));

        self.common().initialize_sft(self.as_sft());
    }

    fn create_space(
        args: &mut CreateSpecificPlanArgs<VM>,
        location: Option<(Address, usize)>,
    ) -> ImmortalSpace<VM> {
        use crate::util::conversions::raw_align_up;
        use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;

        let (vm_space_start, vm_space_bytes) = if let Some((start, size)) = location {
            (start, size)
        } else {
            (
                *args.global_args.options.vm_space_start,
                *args.global_args.options.vm_space_size,
            )
        };

        assert!(!vm_space_start.is_zero());
        assert!(vm_space_bytes > 0);

        // For simplicity, VMSpace has to be outside our available heap range.
        // TODO: Allow VMSpace in our available heap range.
        assert!(!crate::util::heap::layout::range_overlaps_available_range(
            vm_space_start,
            vm_space_bytes
        ));

        let (vm_space_start_aligned, vm_space_bytes_aligned) = (
            vm_space_start.align_down(BYTES_IN_CHUNK),
            raw_align_up(vm_space_bytes, BYTES_IN_CHUNK),
        );
        debug!(
            "start {} is aligned to {}, bytes = {}",
            vm_space_start, vm_space_start_aligned, vm_space_bytes_aligned
        );

        let space_args = args.get_space_args(
            "vm_space",
            false,
            VMRequest::fixed(vm_space_start_aligned, vm_space_bytes_aligned),
        );
        let space =
            ImmortalSpace::new_vm_space(space_args, vm_space_start_aligned, vm_space_bytes_aligned);

        // The space is mapped externally by the VM. We need to update our mmapper to mark the range as mapped.
        space.ensure_mapped();

        space
    }

    fn space_maybe(&self) -> Option<&ImmortalSpace<VM>> {
        self.inner.as_ref()
    }

    fn space(&self) -> &ImmortalSpace<VM> {
        self.inner.as_ref().unwrap()
    }

    // fn space_mut(&mut self) -> &mut ImmortalSpace<VM> {
    //     self.inner.as_mut().unwrap()
    // }

    pub fn prepare(&mut self) {
        if let Some(ref mut space) = &mut self.inner {
            space.prepare()
        }
    }

    pub fn release(&mut self) {
        if let Some(ref mut space) = &mut self.inner {
            space.release()
        }
    }

    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        if let Some(ref space) = &self.inner {
            space.trace_object(queue, object)
        } else {
            panic!("We haven't initialized vm space, but we tried to trace the object {} and thought it was in vm space?", object)
        }
    }
}
