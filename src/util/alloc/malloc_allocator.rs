use crate::{plan::{global::Plan, marksweep::metadata::ALLOC_METADATA_SPEC}, util::{heap::layout::vm_layout_constants::BYTES_IN_CHUNK, side_metadata::load_atomic}};
use crate::plan::marksweep::malloc::ms_calloc;
use crate::plan::marksweep::malloc::ms_malloc_usable_size;
use crate::plan::marksweep::metadata::map_meta_space_for_chunk;
use crate::plan::marksweep::metadata::meta_space_mapped;
use crate::plan::marksweep::metadata::set_alloc_bit;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::conversions;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use atomic::Ordering;
use conversions::chunk_align_down;
use std::{ops::Sub, sync::atomic::AtomicUsize};

pub static mut HEAP_SIZE: usize = 0;
pub static HEAP_USED: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
pub struct MallocAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    space: Option<&'static dyn Space<VM>>,
    plan: &'static dyn Plan<VM = VM>,
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub fn rebind(&mut self, space: Option<&'static dyn Space<VM>>) {
        self.space = space;
    }
}

impl<VM: VMBinding> Allocator<VM> for MallocAllocator<VM> {
    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        self.space
    }
    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }
    fn alloc(&mut self, size: usize, _align: usize, offset: isize) -> Address {
        debug!("alloc");
        debug_assert!(offset == 0);
        unsafe {
            let ptr = ms_calloc(1, size);
            let address = Address::from_mut_ptr(ptr);
            if !meta_space_mapped(address) {
                self.plan.poll(true, self.space.unwrap());
                let chunk_start = conversions::chunk_align_down(address);
                map_meta_space_for_chunk(chunk_start);
            }
            let start = chunk_align_down(address);
            let mut a = start;
            while a < start + BYTES_IN_CHUNK {
                assert!((load_atomic(ALLOC_METADATA_SPEC, a) == 1) == crate::plan::marksweep::metadata::NODES.lock().unwrap().contains(&a.as_usize()), "metadata = {}, nodes = {} for address = {}", (load_atomic(ALLOC_METADATA_SPEC, a) == 1), !(load_atomic(ALLOC_METADATA_SPEC, a) == 1), a );
                a = a.add(8);
            }
            let address_left = address.sub(8);
            let address_right = address.add(8);
            let test_addr = address.sub(56*8);
            debug!("l address = {}, r address = {} t addr = {}", address_left, address_right, test_addr);
            let leftbit_before = load_atomic(ALLOC_METADATA_SPEC, address_left);
            let rightbit_before = load_atomic(ALLOC_METADATA_SPEC, address_right);
            let testbit_before = load_atomic(ALLOC_METADATA_SPEC, test_addr);
            debug!("lb before = {}, rb before = {} t before = {}", leftbit_before, rightbit_before, testbit_before);
            set_alloc_bit(address);
            crate::plan::marksweep::metadata::NODES.lock().unwrap().insert(address.as_usize());
            let leftbit_after = load_atomic(ALLOC_METADATA_SPEC, address_left);
            let rightbit_after = load_atomic(ALLOC_METADATA_SPEC, address_right);
            let testbit_after = load_atomic(ALLOC_METADATA_SPEC, test_addr);
            debug!("lb after = {}, rb after = {} tb after = {}", leftbit_after, rightbit_after, testbit_after);
            assert!(leftbit_after == leftbit_before);
            assert!(rightbit_after == rightbit_before);
            // assert!(testbit_before == testbit_after);
            assert!(load_atomic(ALLOC_METADATA_SPEC, address) == 1);
            a = start;
            while a < start + BYTES_IN_CHUNK {
                if a == test_addr {
                    a = a.add(8);
                }
                assert!((load_atomic(ALLOC_METADATA_SPEC, a) == 1) == crate::plan::marksweep::metadata::NODES.lock().unwrap().contains(&a.as_usize()), "metadata = {}, nodes = {} for address = {}", (load_atomic(ALLOC_METADATA_SPEC, a) == 1), !(load_atomic(ALLOC_METADATA_SPEC, a) == 1), a );
                a = a.add(8);
            }
            HEAP_USED.fetch_add(ms_malloc_usable_size(ptr), Ordering::SeqCst);
            address
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }

    fn alloc_slow_once(&mut self, _size: usize, _align: usize, _offset: isize) -> Address {
        unreachable!();
    }
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static dyn Space<VM>>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        MallocAllocator { tls, space, plan }
    }
}
