use crate::{plan::TransitiveClosure, util::{OpaquePointer, heap::FreeListPageResource}};
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

unsafe impl<VM: VMBinding> Sync for ImmixSpace<VM> {}

const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

pub struct ImmixSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: FreeListPageResource<VM>,
}

impl<VM: VMBinding> SFT for ImmixSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        ForwardingWord::is_forwarded::<VM>(object)
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

impl<VM: VMBinding> Space<VM> for ImmixSpace<VM> {
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
        println!("Init Space {:?}", self as *const _);
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };
        self.pr.bind_space(me);
        self.common().init(self.as_space());
    }
    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immixspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> ImmixSpace<VM> {
    pub fn new(
        name: &'static str,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                zeroed: true,
                vmrequest: VMRequest::discontiguous(),
            },
            vm_map,
            mmapper,
            heap,
        );
        ImmixSpace {
            pr: FreeListPageResource::new_discontiguous(0, vm_map),
            common: UnsafeCell::new(common),
        }
    }

    pub fn defrag_headroom_pages(&self) -> usize {
        self.pr.reserved_pages() * 2 / 100
    }

    pub fn prepare(&self) {
        // TODO: Clear block marks
    }

    pub fn release(&self) {
        // TODO: Release unmarked blocks
    }

    pub fn get_space(&self, tls: OpaquePointer) -> Address {
        self.acquire(tls, 8)
    }

    #[inline]
    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, semantics: AllocationSemantics) -> ObjectReference {
        // trace!("copyspace.trace_object(, {:?}, {:?})", object, semantics,);
        // if !self.from_space() {
        //     return object;
        // }
        // trace!("attempting to forward");
        // let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        // trace!("checking if object is being forwarded");
        // if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
        //     trace!("... yes it is");
        //     let new_object =
        //         ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
        //     trace!("Returning");
        //     new_object
        // } else {
        //     trace!("... no it isn't. Copying");
        //     let new_object =
        //         ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context);
        //     trace!("Forwarding pointer");
        //     trace.process_node(new_object);
        //     trace!("Copying [{:?} -> {:?}]", object, new_object);
        //     new_object
        // }
        unreachable!()
    }

    // #[inline]
    // pub fn trace_object<T: TransitiveClosure, C: CopyContext>(
    //     &self,
    //     trace: &mut T,
    //     object: ObjectReference,
    //     semantics: AllocationSemantics,
    //     copy_context: &mut C,
    // ) -> ObjectReference {
    //     trace!("copyspace.trace_object(, {:?}, {:?})", object, semantics,);
    //     if !self.from_space() {
    //         return object;
    //     }
    //     trace!("attempting to forward");
    //     let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
    //     trace!("checking if object is being forwarded");
    //     if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
    //         trace!("... yes it is");
    //         let new_object =
    //             ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
    //         trace!("Returning");
    //         new_object
    //     } else {
    //         trace!("... no it isn't. Copying");
    //         let new_object =
    //             ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context);
    //         trace!("Forwarding pointer");
    //         trace.process_node(new_object);
    //         trace!("Copying [{:?} -> {:?}]", object, new_object);
    //         new_object
    //     }
    // }
}
