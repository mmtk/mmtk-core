use std::sync::Mutex;

use ::util::heap::PageResource;
use ::util::heap::MonotonePageResource;
use ::util::heap::VMRequest;
use ::util::constants::CARD_META_PAGES_PER_REGION;
use ::util::OpaquePointer;
use ::policy::space::{Space, CommonSpace};
use ::util::{Address, ObjectReference};
use ::plan::TransitiveClosure;
use ::util::forwarding_word as ForwardingWord;
use ::vm::ObjectModel;
use ::vm::VMObjectModel;
use ::plan::Allocator;

use std::cell::UnsafeCell;
use libc::{c_void, mprotect, PROT_NONE, PROT_EXEC, PROT_WRITE, PROT_READ};

const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

#[derive(Debug)]
pub struct CopySpace {
    common: UnsafeCell<CommonSpace<MonotonePageResource<CopySpace>>>,
    from_space: bool,
}

impl Space for CopySpace {
    type PR = MonotonePageResource<CopySpace>;

    fn common(&self) -> &CommonSpace<Self::PR> {
        unsafe { &*self.common.get() }
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<Self::PR> {
        &mut *self.common.get()
    }

    fn init(&mut self) {
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };

        let common_mut = self.common_mut();
        if common_mut.vmrequest.is_discontiguous() {
            common_mut.pr = Some(MonotonePageResource::new_discontiguous(
                META_DATA_PAGES_PER_REGION));
        } else {
            common_mut.pr = Some(MonotonePageResource::new_contiguous(common_mut.start,
                                                                      common_mut.extent,
                                                                      META_DATA_PAGES_PER_REGION));
        }
        common_mut.pr.as_mut().unwrap().bind_space(me);
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        ForwardingWord::is_forwarded(object)
    }

    fn is_movable(&self) -> bool {
        true
    }

    fn release_multiple_pages(&mut self, start: Address) {
        panic!("copyspace only releases pages enmasse")
    }
}

impl CopySpace {
    pub fn new(name: &'static str, from_space: bool, zeroed: bool, vmrequest: VMRequest) -> Self {
        CopySpace {
            common: UnsafeCell::new(CommonSpace::new(name, true, false, zeroed, vmrequest)),
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
        let mut forwarding_word = ForwardingWord::attempt_to_forward(object);
        trace!("checking if object is being forwarded");
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_word) {
            trace!("... yes it is");
            while ForwardingWord::state_is_being_forwarded(forwarding_word) {
                forwarding_word = VMObjectModel::read_available_bits_word(object);
            }
            trace!("Returning");
            return ForwardingWord::extract_forwarding_pointer(forwarding_word);
        } else {
            trace!("... no it isn't. Copying");
            let new_object = VMObjectModel::copy(object, allocator, tls);
            trace!("Setting forwarding pointer");
            ForwardingWord::set_forwarding_pointer(object, new_object);
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
        let start = self.common().start.as_usize();
        let extent = self.common().extent;
        unsafe {
            mprotect(start as *mut c_void, extent, PROT_NONE);
        }
        trace!("Protect {:x} {:x}", start, start + extent);
    }

    pub fn unprotect(&self) {
        if !self.common().contiguous {
            panic!("Implement Options.protectOnRelease for MonotonePageResource.release_pages_extent")
        }
        let start = self.common().start.as_usize();
        let extent = self.common().extent;
        unsafe {
            mprotect(start as *mut c_void, extent, PROT_READ | PROT_WRITE | PROT_EXEC);
        }
        trace!("Unprotect {:x} {:x}", start, start + extent);
    }
}