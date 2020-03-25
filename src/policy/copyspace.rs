use ::util::heap::PageResource;
use ::util::heap::MonotonePageResource;
use ::util::heap::VMRequest;
use ::util::constants::CARD_META_PAGES_PER_REGION;
use ::util::OpaquePointer;
use ::policy::space::{Space, CommonSpace};
use ::util::{Address, ObjectReference};
use ::plan::TransitiveClosure;
use ::util::forwarding_word as ForwardingWord;
use ::plan::Allocator;

use std::cell::UnsafeCell;
use libc::{mprotect, PROT_NONE, PROT_EXEC, PROT_WRITE, PROT_READ};
use util::heap::layout::heap_layout::{VMMap, Mmapper};
use util::heap::HeapMeta;
use vm::VMBinding;

const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

pub struct CopySpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM, MonotonePageResource<VM, CopySpace<VM>>>>,
    from_space: bool,
}

impl<VM: VMBinding> Space<VM> for CopySpace<VM> {
    type PR = MonotonePageResource<VM, CopySpace<VM>>;

    fn common(&self) -> &CommonSpace<VM, Self::PR> {
        unsafe { &*self.common.get() }
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM, Self::PR> {
        &mut *self.common.get()
    }

    fn init(&mut self, vm_map: &'static VMMap) {
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };

        let common_mut = self.common_mut();
        if common_mut.vmrequest.is_discontiguous() {
            common_mut.pr = Some(MonotonePageResource::new_discontiguous(
                META_DATA_PAGES_PER_REGION, vm_map));
        } else {
            common_mut.pr = Some(MonotonePageResource::new_contiguous(common_mut.start,
                                                                      common_mut.extent,
                                                                      META_DATA_PAGES_PER_REGION,
                                                                      vm_map));
        }
        common_mut.pr.as_mut().unwrap().bind_space(me);
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        ForwardingWord::is_forwarded::<VM>(object)
    }

    fn is_movable(&self) -> bool {
        true
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("copyspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> CopySpace<VM> {
    pub fn new(name: &'static str, from_space: bool, zeroed: bool, vmrequest: VMRequest, vm_map: &'static VMMap, mmapper: &'static Mmapper, heap: &mut HeapMeta) -> Self {
        CopySpace {
            common: UnsafeCell::new(CommonSpace::new(name, true, false, zeroed, vmrequest, vm_map, mmapper, heap)),
            from_space,
        }
    }

    pub fn prepare(&mut self, from_space: bool) {
        self.from_space = from_space;
    }

    pub unsafe fn release(&mut self) {
        self.common().pr.as_ref().unwrap().reset();
        self.from_space = false;
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        allocator: Allocator,
        tls: OpaquePointer,
    ) -> ObjectReference
    {
        trace!("copyspace.trace_object(, {:?}, {:?}, {:?})", object, allocator, tls);
        if !self.from_space {
            return object;
        }
        trace!("attempting to forward");
        let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        trace!("checking if object is being forwarded");
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
            trace!("... yes it is");
            let new_object = ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
            trace!("Returning");
            return new_object;
        } else {
            trace!("... no it isn't. Copying");
            let new_object = ForwardingWord::forward_object::<VM>(object, allocator, tls);
            trace!("Forwarding pointer");
            trace.process_node(new_object);
            trace!("Copying [{:?} -> {:?}]", object, new_object);
            return new_object;
        }
    }

    pub fn protect(&self) {
        if !self.common().contiguous {
            panic!("Implement Options.protectOnRelease for MonotonePageResource.release_pages_extent")
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
            panic!("Implement Options.protectOnRelease for MonotonePageResource.release_pages_extent")
        }
        let start = self.common().start;
        let extent = self.common().extent;
        unsafe {
            mprotect(start.to_mut_ptr(), extent, PROT_READ | PROT_WRITE | PROT_EXEC);
        }
        trace!("Unprotect {:x} {:x}", start, start + extent);
    }
}