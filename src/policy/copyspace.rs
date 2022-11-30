use crate::plan::{ObjectQueue, VectorObjectQueue};
use crate::policy::copy_context::PolicyCopyContext;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space};
use crate::scheduler::GCWorker;
use crate::util::copy::*;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
#[cfg(feature = "global_alloc_bit")]
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::metadata::{extract_side_metadata, MetadataSpec};
use crate::util::object_forwarding;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use libc::{mprotect, PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::sync::atomic::{AtomicBool, Ordering};

/// This type implements a simple copying space.
pub struct CopySpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
    from_space: AtomicBool,
}

impl<VM: VMBinding> SFT for CopySpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        !self.is_from_space() || object_forwarding::is_forwarded::<VM>(object)
    }

    fn is_movable(&self) -> bool {
        true
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        !self.is_from_space()
    }

    fn initialize_object_metadata(&self, _object: ObjectReference, _alloc: bool) {
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::set_alloc_bit::<VM>(_object);
    }

    #[inline(always)]
    fn get_forwarded_object(&self, object: ObjectReference) -> Option<ObjectReference> {
        if !self.is_from_space() {
            return None;
        }

        if object_forwarding::is_forwarded::<VM>(object) {
            Some(object_forwarding::read_forwarding_pointer::<VM>(object))
        } else {
            None
        }
    }

    #[cfg(feature = "is_mmtk_object")]
    #[inline(always)]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        crate::util::alloc_bit::is_alloced_object::<VM>(addr).is_some()
    }

    #[inline(always)]
    fn sft_trace_object(
        &self,
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        worker: GCWorkerMutRef,
    ) -> ObjectReference {
        let worker = worker.into_mut::<VM>();
        self.trace_object(queue, object, self.common.copy, worker)
    }
}

impl<VM: VMBinding> Space<VM> for CopySpace<VM> {
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
        panic!("copyspace only releases pages enmasse")
    }

    fn set_copy_for_sft_trace(&mut self, semantics: Option<CopySemantics>) {
        self.common.copy = semantics;
    }
}

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for CopySpace<VM> {
    #[inline(always)]
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        copy: Option<CopySemantics>,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.trace_object(queue, object, copy, worker)
    }

    #[inline(always)]
    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        true
    }
}

impl<VM: VMBinding> CopySpace<VM> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'static str,
        from_space: bool,
        zeroed: bool,
        vmrequest: VMRequest,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        let local_specs = extract_side_metadata(&[
            *VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
            *VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
        ]);
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                needs_log_bit: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: local_specs,
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        CopySpace {
            pr: if vmrequest.is_discontiguous() {
                MonotonePageResource::new_discontiguous(vm_map)
            } else {
                MonotonePageResource::new_contiguous(common.start, common.extent, vm_map)
            },
            common,
            from_space: AtomicBool::new(from_space),
        }
    }

    pub fn prepare(&self, from_space: bool) {
        self.from_space.store(from_space, Ordering::SeqCst);
        // Clear the metadata if we are using side forwarding status table. Otherwise
        // objects may inherit forwarding status from the previous GC.
        // TODO: Fix performance.
        if let MetadataSpec::OnSide(side_forwarding_status_table) =
            *<VM::VMObjectModel as ObjectModel<VM>>::LOCAL_FORWARDING_BITS_SPEC
        {
            side_forwarding_status_table
                .bzero_metadata(self.common.start, self.pr.cursor() - self.common.start);
        }
    }

    pub fn release(&self) {
        unsafe {
            #[cfg(feature = "global_alloc_bit")]
            self.reset_alloc_bit();
            self.pr.reset();
        }
        self.common.metadata.reset();
        self.from_space.store(false, Ordering::SeqCst);
    }

    #[cfg(feature = "global_alloc_bit")]
    unsafe fn reset_alloc_bit(&self) {
        let current_chunk = self.pr.get_current_chunk();
        if self.common.contiguous {
            // If we have allocated something into this space, we need to clear its alloc bit.
            if current_chunk != self.common.start {
                crate::util::alloc_bit::bzero_alloc_bit(
                    self.common.start,
                    current_chunk + BYTES_IN_CHUNK - self.common.start,
                );
            }
        } else {
            unimplemented!();
        }
    }

    fn is_from_space(&self) -> bool {
        self.from_space.load(Ordering::SeqCst)
    }

    #[inline(always)]
    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        semantics: Option<CopySemantics>,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        trace!("copyspace.trace_object(, {:?}, {:?})", object, semantics,);

        // If this is not from space, we do not need to trace it (the object has been copied to the tosapce)
        if !self.is_from_space() {
            // The copy semantics for tospace should be none.
            return object;
        }

        // This object is in from space, we will copy. Make sure we have a valid copy semantic.
        debug_assert!(semantics.is_some());

        #[cfg(feature = "global_alloc_bit")]
        debug_assert!(
            crate::util::alloc_bit::is_alloced::<VM>(object),
            "{:x}: alloc bit not set",
            object
        );

        trace!("attempting to forward");
        let forwarding_status = object_forwarding::attempt_to_forward::<VM>(object);

        trace!("checking if object is being forwarded");
        if object_forwarding::state_is_forwarded_or_being_forwarded(forwarding_status) {
            trace!("... yes it is");
            let new_object =
                object_forwarding::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
            trace!("Returning");
            new_object
        } else {
            trace!("... no it isn't. Copying");
            let new_object = object_forwarding::forward_object::<VM>(
                object,
                semantics.unwrap(),
                worker.get_copy_context_mut(),
            );
            trace!("Forwarding pointer");
            queue.enqueue(new_object);
            trace!("Copied [{:?} -> {:?}]", object, new_object);
            new_object
        }
    }

    #[allow(dead_code)] // Only used with certain features (such as sanity)
    pub fn protect(&self) {
        if !self.common().contiguous {
            panic!(
                "Implement Options.protectOnRelease for MonotonePageResource.release_pages_extent"
            )
        }
        let start = self.common().start;
        let extent = self.common().extent;
        unsafe {
            mprotect(start.to_mut_ptr(), extent, PROT_NONE);
        }
        trace!("Protect {:x} {:x}", start, start + extent);
    }

    #[allow(dead_code)] // Only used with certain features (such as sanity)
    pub fn unprotect(&self) {
        if !self.common().contiguous {
            panic!(
                "Implement Options.protectOnRelease for MonotonePageResource.release_pages_extent"
            )
        }
        let start = self.common().start;
        let extent = self.common().extent;
        unsafe {
            mprotect(
                start.to_mut_ptr(),
                extent,
                PROT_READ | PROT_WRITE | PROT_EXEC,
            );
        }
        trace!("Unprotect {:x} {:x}", start, start + extent);
    }
}

use crate::plan::Plan;
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::opaque_pointer::VMWorkerThread;

/// Copy allocator for CopySpace
pub struct CopySpaceCopyContext<VM: VMBinding> {
    copy_allocator: BumpAllocator<VM>,
}

impl<VM: VMBinding> PolicyCopyContext for CopySpaceCopyContext<VM> {
    type VM = VM;

    fn prepare(&mut self) {}

    fn release(&mut self) {}

    #[inline(always)]
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
    ) -> Address {
        self.copy_allocator.alloc(bytes, align, offset)
    }
}

impl<VM: VMBinding> CopySpaceCopyContext<VM> {
    pub fn new(
        tls: VMWorkerThread,
        plan: &'static dyn Plan<VM = VM>,
        tospace: &'static CopySpace<VM>,
    ) -> Self {
        CopySpaceCopyContext {
            copy_allocator: BumpAllocator::new(tls.0, tospace, plan),
        }
    }
}

impl<VM: VMBinding> CopySpaceCopyContext<VM> {
    pub fn rebind(&mut self, space: &CopySpace<VM>) {
        self.copy_allocator
            .rebind(unsafe { &*{ space as *const _ } });
    }
}
