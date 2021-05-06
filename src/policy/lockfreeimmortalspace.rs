use crate::mmtk::SFT_MAP;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::address::Address;
use crate::util::conversions::bytes_to_chunks_up;
use crate::util::heap::PageResource;
use crate::util::side_metadata::{SideMetadata, SideMetadataContext, SideMetadataSpec};

use crate::util::ObjectReference;

use crate::util::conversions;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{
    AVAILABLE_BYTES, AVAILABLE_END, AVAILABLE_START,
};
use crate::util::opaque_pointer::OpaquePointer;
use crate::vm::VMBinding;
use crate::vm::*;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LockFreeImmortalSpace<VM: VMBinding> {
    #[allow(unused)]
    name: &'static str,
    /// Heap range start
    ///
    /// We use `AtomicUsize` instead of `Address` here to atomically bumping this cursor.
    /// TODO: Better address type here (Atomic<Address>?)
    cursor: AtomicUsize,
    /// Heap range end
    limit: Address,
    /// Zero memory after slow-path allocation
    slow_path_zeroing: bool,
    metadata: SideMetadata,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> SFT for LockFreeImmortalSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
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
    fn initialize_header(&self, _object: ObjectReference, _alloc: bool) {
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

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }

    fn init(&mut self, _vm_map: &'static VMMap) {
        let total_pages = VM::VMActivePlan::global()
            .base()
            .heap
            .total_pages
            .load(Ordering::SeqCst);
        let total_bytes = conversions::pages_to_bytes(total_pages);
        assert!(total_pages > 0);
        assert!(
            total_bytes <= AVAILABLE_BYTES,
            "Initial requested memory ({} bytes) overflows the heap. Max heap size is {} bytes.",
            total_bytes,
            AVAILABLE_BYTES
        );
        self.limit = AVAILABLE_START + total_bytes;
        // Eagerly memory map the entire heap (also zero all the memory)
        crate::util::memory::dzmmap_noreplace(AVAILABLE_START, total_bytes).unwrap();
        if self
            .metadata
            .try_map_metadata_space(AVAILABLE_START, total_bytes)
            .is_err()
        {
            // TODO(Javad): handle meta space allocation failure
            panic!("failed to mmap meta memory");
        }
        SFT_MAP.update(
            self.as_sft(),
            AVAILABLE_START,
            bytes_to_chunks_up(total_bytes),
        );
    }

    fn reserved_pages(&self) -> usize {
        let cursor = unsafe { Address::from_usize(self.cursor.load(Ordering::Relaxed)) };
        conversions::bytes_to_pages_up(self.limit - cursor) + self.metadata.reserved_pages()
    }

    fn acquire(&self, _tls: OpaquePointer, pages: usize) -> Address {
        let bytes = conversions::pages_to_bytes(pages);
        let start = unsafe { Address::from_usize(self.cursor.fetch_add(bytes, Ordering::Relaxed)) };
        if start + bytes > self.limit {
            panic!("OutOfMemory")
        }
        if self.slow_path_zeroing {
            crate::util::memory::zero(start, bytes);
        }
        start
    }
}

impl<VM: VMBinding> LockFreeImmortalSpace<VM> {
    pub fn new(
        name: &'static str,
        slow_path_zeroing: bool,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
    ) -> Self {
        Self {
            name,
            cursor: AtomicUsize::new(AVAILABLE_START.as_usize()),
            limit: AVAILABLE_END,
            slow_path_zeroing,
            metadata: SideMetadata::new(SideMetadataContext {
                global: global_side_metadata_specs,
                local: vec![],
            }),
            phantom: PhantomData,
        }
    }
}
