use crate::plan::TransitiveClosure;
use crate::plan::{AllocationSemantics, CopyContext};
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::constants::CARD_META_PAGES_PER_REGION;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
#[cfg(feature = "global_alloc_bit")]
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::metadata::{extract_side_metadata, side_metadata, MetadataSpec};
use crate::util::object_forwarding;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use libc::{mprotect, PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::sync::atomic::{AtomicBool, Ordering};

const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

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
        !self.from_space() || object_forwarding::is_forwarded::<VM>(object)
    }
    fn is_movable(&self) -> bool {
        true
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        !self.from_space()
    }
    fn initialize_object_metadata(&self, _object: ObjectReference, _alloc: bool) {
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::set_alloc_bit(_object);
    }
    #[inline(always)]
    fn get_forwarded_object(&self, object: ObjectReference) -> Option<ObjectReference> {
        if !self.from_space() {
            return None;
        }
        if object_forwarding::is_forwarded::<VM>(object) {
            Some(object_forwarding::read_forwarding_pointer::<VM>(object))
        } else {
            None
        }
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

    fn init(&mut self, _vm_map: &'static VMMap) {
        self.common().init(self.as_space());
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("copyspace only releases pages enmasse")
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
                MonotonePageResource::new_discontiguous(META_DATA_PAGES_PER_REGION, vm_map)
            } else {
                MonotonePageResource::new_contiguous(
                    common.start,
                    common.extent,
                    META_DATA_PAGES_PER_REGION,
                    vm_map,
                )
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
            side_metadata::bzero_metadata(
                &side_forwarding_status_table,
                self.common.start,
                self.pr.cursor() - self.common.start,
            );
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
            crate::util::alloc_bit::bzero_alloc_bit(
                self.common.start,
                current_chunk + BYTES_IN_CHUNK - self.common.start,
            );
        } else {
            unimplemented!();
        }
    }

    fn from_space(&self) -> bool {
        self.from_space.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn trace_object<T: TransitiveClosure, C: CopyContext>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        semantics: AllocationSemantics,
        copy_context: &mut C,
    ) -> ObjectReference {
        trace!("copyspace.trace_object(, {:?}, {:?})", object, semantics,);
        if !self.from_space() {
            return object;
        }
        #[cfg(feature = "global_alloc_bit")]
        debug_assert!(
            crate::util::alloc_bit::is_alloced(object),
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
            let new_object =
                object_forwarding::forward_object::<VM, _>(object, semantics, copy_context);
            trace!("Forwarding pointer");
            trace.process_node(new_object);
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
