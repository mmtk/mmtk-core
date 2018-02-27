use std::sync::Mutex;

use ::util::heap::PageResource;
use ::util::heap::MonotonePageResource;
use ::util::heap::VMRequest;
use ::util::constants::CARD_META_PAGES_PER_REGION;

use ::policy::space::{Space, CommonSpace};
use ::util::{Address, ObjectReference};
use ::plan::TransitiveClosure;
use ::util::forwarding_word as ForwardingWord;
use ::vm::ObjectModel;
use ::vm::VMObjectModel;
use ::plan::Allocator;

use std::cell::UnsafeCell;

const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

pub struct CopySpace {
    common: UnsafeCell<CommonSpace<CopySpace, MonotonePageResource<CopySpace>>>,
    from_space: bool,
}

impl Space<MonotonePageResource<CopySpace>> for CopySpace {
    fn common(&self) -> &CommonSpace<CopySpace, MonotonePageResource<CopySpace>> {
        unsafe{&*self.common.get()}
    }

    fn common_mut(&self) -> &mut CommonSpace<CopySpace, MonotonePageResource<CopySpace>> {
        unsafe{&mut *self.common.get()}
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

    pub fn release(&self) {
        unimplemented!()
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        allocator: Allocator,
        thread_id: usize,
    ) -> ObjectReference
    {
        if !self.from_space {
            return object;
        }
        let mut forwarding_word = ForwardingWord::attempt_to_forward(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_word) {
            while ForwardingWord::state_is_being_forwarded(forwarding_word) {
                forwarding_word = VMObjectModel::read_available_bits_word(object);
            }
            return ForwardingWord::extract_forwarding_pointer(forwarding_word);
        } else {
            let new_object = VMObjectModel::copy(object, allocator, thread_id);
            ForwardingWord::set_forwarding_pointer(object, new_object);
            trace.process_node(new_object);
            return new_object;
        }
    }
}