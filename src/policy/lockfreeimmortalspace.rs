use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::address::Address;
use crate::util::heap::{MonotonePageResource, PageResource, VMRequest};

use crate::util::constants::CARD_META_PAGES_PER_REGION;
use crate::util::ObjectReference;

use crate::plan::TransitiveClosure;
use crate::util::header_byte;

use crate::policy::space::SpaceOptions;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::vm::VMBinding;
use std::cell::UnsafeCell;
use crate::vm::*;
use std::marker::PhantomData;
use crate::util::opaque_pointer::OpaquePointer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use crate::plan::Plan;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::{AVAILABLE_START, AVAILABLE_END, AVAILABLE_BYTES};

pub struct LockFreeImmortalSpace<VM: VMBinding> {
    name: &'static str,
    cursor: AtomicUsize,
    limit: Address,
    zeroed: bool,
    phantom: PhantomData<VM>,
}

unsafe impl<VM: VMBinding> Sync for LockFreeImmortalSpace<VM> {}

const GC_MARK_BIT_MASK: u8 = 1;
const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

impl<VM: VMBinding> SFT for LockFreeImmortalSpace<VM> {
    fn is_live(&self, _object: ObjectReference) -> bool {
        unimplemented!()
    }
    fn is_movable(&self) -> bool {
        unimplemented!()
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        unimplemented!()
    }
    fn initialize_header(&self, object: ObjectReference, _alloc: bool) {
        unimplemented!()
    }
}

impl<VM: VMBinding> Space<VM> for LockFreeImmortalSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        unimplemented!()
    }
    fn common(&self) -> &CommonSpace<VM> {
        unimplemented!()
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        unimplemented!()
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }

    fn init(&mut self, _vm_map: &'static VMMap) {
        let total_pages = <VM as VMBinding>::VMActivePlan::global().base().heap.total_pages.load(Ordering::SeqCst);
        let total_bytes = conversions::pages_to_bytes(total_pages);
        assert!(total_pages > 0);
        assert!(total_bytes <= AVAILABLE_BYTES, "Initial requested memory ({} bytes) overflows the heap. Max heap size is {} bytes.", total_bytes, AVAILABLE_BYTES);
        self.limit = AVAILABLE_START + total_bytes;
        crate::util::memory::dzmmap(AVAILABLE_START, total_bytes).unwrap();
    }

    fn reserved_pages(&self) -> usize {
        let cursor = unsafe { Address::from_usize(self.cursor.load(Ordering::Relaxed)) };
        conversions::bytes_to_pages_up(self.limit - cursor)
    }

    fn acquire(&self, tls: OpaquePointer, pages: usize) -> Address {
        let bytes = conversions::pages_to_bytes(pages);
        let start = {
            let start = unsafe { Address::from_usize(self.cursor.fetch_add(bytes, Ordering::Relaxed)) };
            if start + bytes <= self.limit {
                start
            } else {
                panic!("OutOfMemory")
            }
        };
        if self.zeroed {
            crate::util::memory::zero(start, bytes);
        }
        start
    }
}

impl<VM: VMBinding> LockFreeImmortalSpace<VM> {
    pub fn new(name: &'static str, zeroed: bool) -> Self {
        Self {
            name,
            cursor: AtomicUsize::new(AVAILABLE_START.as_usize()),
            limit: AVAILABLE_END,
            zeroed,
            phantom: PhantomData,
        }
    }
}
