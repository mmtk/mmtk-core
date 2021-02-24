use super::metadata::*;
use crate::plan::TransitiveClosure;
use crate::policy::space::CommonSpace;
use crate::policy::space::SFT;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::PageResource;
use crate::util::heap::{layout::vm_layout_constants::PAGES_IN_CHUNK, MonotonePageResource};
use crate::util::malloc::*;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::conversions;
use crate::vm::VMBinding;
use crate::vm::{ActivePlan, ObjectModel};
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::mmapper::Mmapper as IMmapper;
use crate::{
    policy::space::Space,
    util::{heap::layout::vm_layout_constants::BYTES_IN_CHUNK, side_metadata::load_atomic},
};
use std::{collections::HashSet, marker::PhantomData};
use std::collections::LinkedList;
use std::sync::atomic::AtomicUsize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

const ASSERT_ALLOCATION: bool = cfg!(debug_assertions) && true;

pub struct MallocSpace<VM: VMBinding> {
    phantom: PhantomData<VM>,
    active_bytes: AtomicUsize,
    // Mapping between allocated address and its size - this is used to check correctness.
    #[cfg(debug_assertions)]
    active_mem: Mutex<HashMap<Address, usize>>,
}

impl<VM: VMBinding> SFT for MallocSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    fn is_live(&self, _object: ObjectReference) -> bool {
        unimplemented!();
    }
    fn is_movable(&self) -> bool {
        false
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_header(&self, object: ObjectReference, _alloc: bool) {
        set_alloc_bit(object.to_address());
    }
}

impl<VM: VMBinding> Space<VM> for MallocSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        unreachable!()
    }
    fn common(&self) -> &CommonSpace<VM> {
        unreachable!()
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        unreachable!()
    }

    fn init(&mut self, _vm_map: &'static VMMap) {
        // Do nothing
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        unreachable!()
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        let address = object.to_address();
        self.address_in_space(address)
    }

    fn address_in_space(&self, start: Address) -> bool {
        is_meta_space_mapped(start) && load_atomic(ALLOC_METADATA_SPEC, start) == 1
    }

    fn get_name(&self) -> &'static str {
        "MallocSpace"
    }

    fn reserved_pages(&self) -> usize {
        conversions::bytes_to_pages_up(self.active_bytes.load(Ordering::SeqCst))
    }

    unsafe fn release_all_chunks(&self) {
        let mut released_chunks: HashSet<Address> = HashSet::new();

        // To sum up the total size of live objects. We check this against the active_bytes we maintain.
        #[cfg(debug_assertions)]
        let mut live_bytes = 0;

        debug!("Used bytes before releasing: {}", self.active_bytes.load(Ordering::Relaxed));

        for chunk_start in ACTIVE_CHUNKS.read().unwrap().iter() {
            debug!("Check active chunk {:?}", chunk_start);
            let mut chunk_is_empty = true;
            let mut address = *chunk_start;
            let chunk_end = chunk_start.add(BYTES_IN_CHUNK);

            // Linear scan through the chunk
            while address < chunk_end {
                if load_atomic(ALLOC_METADATA_SPEC, address) == 1 {
                    // We know it is an object
                    let object = unsafe { address.to_object_reference() };

                    #[cfg(debug_assertions)]
                    if ASSERT_ALLOCATION {
                        let obj_start = VM::VMObjectModel::object_start_ref(object);
                        let ptr = VM::VMObjectModel::object_start_ref(object).to_mut_ptr();
                        let bytes = malloc_usable_size(ptr);

                        debug_assert!(self.active_mem.lock().unwrap().contains_key(&obj_start), "Object with alloc bit is not in active_mem");
                        debug_assert_eq!(self.active_mem.lock().unwrap().get(&obj_start), Some(&bytes), "Object size in active_mem does not match the size from malloc_usable_size");
                    }

                    if !is_marked(address) {
                        // Dead object
                        trace!("Address {} has alloc bit but no mark bit, it is dead. ", address);

                        // Get the start address of the object, and free it
                        self.free(VM::VMObjectModel::object_start_ref(object));
                        unset_alloc_bit(object.to_address());
                    } else {
                        // Live object. Unset mark bit
                        unset_mark_bit(address);
                        // This chunk is still active.
                        chunk_is_empty = false;

                        #[cfg(debug_assertions)]
                        {
                            // Accumulate live bytes
                            live_bytes += malloc_usable_size(VM::VMObjectModel::object_start_ref(object).to_mut_ptr());
                        }
                    }
                }
                address = address.add(VM::MIN_ALIGNMENT);
            }
            if chunk_is_empty {
                debug!("Release malloc chunk {} to {}", chunk_start, *chunk_start + BYTES_IN_CHUNK);
                released_chunks.insert(*chunk_start);
            }
        }

        debug!("Used bytes after releasing: {}", self.active_bytes.load(Ordering::SeqCst));
        #[cfg(debug_assertions)]
        debug_assert_eq!(live_bytes, self.active_bytes.load(Ordering::SeqCst));

        ACTIVE_CHUNKS
            .write()
            .unwrap()
            .retain(|c| !released_chunks.contains(&*c));
    }
}

impl<VM: VMBinding> MallocSpace<VM> {
    pub fn new(vm_map: &'static VMMap) -> Self {
        MallocSpace {
            phantom: PhantomData,
            active_bytes: AtomicUsize::new(0),
            #[cfg(debug_assertions)]
            active_mem: Mutex::new(HashMap::new()),
        }
    }

    pub fn alloc(&self, size: usize) -> Address {
        let raw = unsafe { calloc(1, size) };
        let actual_size = unsafe { malloc_usable_size(raw) };
        let address = Address::from_mut_ptr(raw);

        if !address.is_zero() {
            if !is_meta_space_mapped(address) {
                VM::VMActivePlan::global().poll(false, self);
                let chunk_start = conversions::chunk_align_down(address);
                debug!("Add active malloc chunk {} to {}", chunk_start, chunk_start + BYTES_IN_CHUNK);
                map_meta_space_for_chunk(chunk_start);
            }
            self.active_bytes.fetch_add(actual_size, Ordering::SeqCst);

            #[cfg(debug_assertions)]
            if ASSERT_ALLOCATION {
                self.active_mem.lock().unwrap().insert(address, actual_size);
            }
        }
        address
    }

    pub fn free(&self, addr: Address) {
        let ptr = addr.to_mut_ptr();
        let bytes = unsafe { malloc_usable_size(ptr) };
        trace!("Free memory {:?}", ptr);
        unsafe { free(ptr); }
        self.active_bytes.fetch_sub(bytes, Ordering::SeqCst);

        #[cfg(debug_assertions)]
        self.active_mem.lock().unwrap().remove(&addr);
    }

    #[inline]
    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let address = object.to_address();
        assert!(
            self.address_in_space(address),
            "Cannot mark an object {} that was not alloced by malloc.",
            address,
        );
        if !is_marked(address) {
            set_mark_bit(address);
            trace.process_node(object);
        }
        object
    }
}
