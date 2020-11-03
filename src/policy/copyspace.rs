use crate::plan::TransitiveClosure;
use crate::plan::{AllocationSemantics, CopyContext};
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::constants::CARD_META_PAGES_PER_REGION;
use crate::util::forwarding_word as ForwardingWord;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use libc::{mprotect, PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, Ordering};

unsafe impl<VM: VMBinding> Sync for CopySpace<VM> {}

const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

pub struct CopySpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: MonotonePageResource<VM>,
    from_space: AtomicBool,
}

impl<VM: VMBinding> SFT for CopySpace<VM> {
    fn is_live(&self, object: ObjectReference) -> bool {
        !self.from_space() || ForwardingWord::is_forwarded::<VM>(object)
    }
    fn is_movable(&self) -> bool {
        true
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        !self.from_space()
    }
    fn initialize_header(&self, _object: ObjectReference, _alloc: bool) {}
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
        unsafe { &*self.common.get() }
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        &mut *self.common.get()
    }

    fn init(&mut self, _vm_map: &'static VMMap) {
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };
        self.pr.bind_space(me);
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("copyspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> CopySpace<VM> {
    pub fn new(
        name: &'static str,
        from_space: bool,
        zeroed: bool,
        vmrequest: VMRequest,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                zeroed,
                vmrequest,
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
            common: UnsafeCell::new(common),
            from_space: AtomicBool::new(from_space),
        }
    }

    pub fn prepare(&self, from_space: bool) {
        self.from_space.store(from_space, Ordering::SeqCst);
    }

    pub fn release(&self) {
        unsafe {
            self.pr.reset();
        }
        self.from_space.store(false, Ordering::SeqCst);
    }

    fn from_space(&self) -> bool {
        self.from_space.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        semantics: AllocationSemantics,
        copy_context: &mut impl CopyContext,
    ) -> ObjectReference {
        trace!("copyspace.trace_object(, {:?}, {:?})", object, semantics,);
        if !self.from_space() {
            return object;
        }
        trace!("attempting to forward");
        let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        trace!("checking if object is being forwarded");
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
            trace!("... yes it is");
            let new_object =
                ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
            trace!("Returning");
            new_object
        } else {
            trace!("... no it isn't. Copying");
            let new_object =
                ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context);
            trace!("Forwarding pointer");
            trace.process_node(new_object);
            trace!("Copying [{:?} -> {:?}]", object, new_object);
            new_object
        }
    }

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
