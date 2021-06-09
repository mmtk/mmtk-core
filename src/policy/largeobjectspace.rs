use crate::plan::TransitiveClosure;
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::header_byte::HeaderByte;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::heap::{FreeListPageResource, PageResource, VMRequest};
use crate::util::opaque_pointer::*;
#[cfg(target_pointer_width = "32")]
use crate::util::side_metadata::meta_bytes_per_chunk;
#[cfg(target_pointer_width = "64")]
use crate::util::side_metadata::LOCAL_SIDE_METADATA_BASE_ADDRESS;
use crate::util::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::treadmill::TreadMill;
use crate::util::{Address, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use crate::{
    plan::PlanConstraints,
    util::{
        constants,
        side_metadata::{self, SideMetadataScope},
    },
};

const MARK_BIT: u8 = 0b01;
/// This type implements a policy for large objects. Each instance corresponds
/// to one Treadmill space.
pub struct LargeObjectSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    mark_state: u8,
    in_nursery_gc: bool,
    treadmill: TreadMill,
    header_byte: HeaderByte,
}

impl<VM: VMBinding> SFT for LargeObjectSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        self.test_mark_bit(object, self.mark_state)
    }
    fn is_movable(&self) -> bool {
        false
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_object_metadata(&self, object: ObjectReference, alloc: bool) {
        let cell = VM::VMObjectModel::object_start_ref(object);
        side_metadata::store_atomic(Self::NURSERY_STATE, cell, if alloc { 1 } else { 0 });
        side_metadata::store_atomic(Self::MARK_TABLE, cell, self.mark_state as _);
        self.treadmill.add_to_treadmill(cell, alloc);
        // TODO: logging bit should move to (global) side-metadata.
        debug_assert!(!self.header_byte.needs_unlogged_bit);
        debug_assert!(object.is_live());
    }
}

impl<VM: VMBinding> Space<VM> for LargeObjectSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }
    fn init(&mut self, _vm_map: &'static VMMap) {}

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, start: Address) {
        self.pr.release_pages(start);
    }
}

impl<VM: VMBinding> LargeObjectSpace<VM> {
    #[cfg(target_pointer_width = "64")]
    const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
        log_num_of_bits: 0,
        log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as _,
    };
    #[cfg(target_pointer_width = "64")]
    const NURSERY_STATE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: Self::MARK_TABLE.offset
            + side_metadata::metadata_address_range_size(Self::MARK_TABLE),
        log_num_of_bits: 0,
        log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as _,
    };

    #[cfg(target_pointer_width = "32")]
    pub(super) const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: 0,
        log_num_of_bits: 0,
        log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as _,
    };
    #[cfg(target_pointer_width = "32")]
    pub(super) const NURSERY_STATE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: Self::MARK_TABLE.offset
            + meta_bytes_per_chunk(
                Self::MARK_TABLE.log_min_obj_size,
                Self::MARK_TABLE.log_num_of_bits,
            ),
        log_num_of_bits: 0,
        log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as _,
    };

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
        constraints: &'static PlanConstraints,
        protect_memory_on_release: bool,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: vec![Self::MARK_TABLE, Self::NURSERY_STATE],
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        let mut pr = if vmrequest.is_discontiguous() {
            FreeListPageResource::new_discontiguous(0, vm_map)
        } else {
            FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
        };
        pr.protect_memory_on_release = protect_memory_on_release;
        LargeObjectSpace {
            pr,
            common,
            mark_state: 0,
            in_nursery_gc: false,
            treadmill: TreadMill::new(),
            header_byte: HeaderByte::new(constraints),
        }
    }

    pub fn prepare(&mut self, full_heap: bool) {
        if full_heap {
            debug_assert!(self.treadmill.from_space_empty());
            self.mark_state = MARK_BIT - self.mark_state;
        }
        self.treadmill.flip(full_heap);
        self.in_nursery_gc = !full_heap;
    }

    pub fn release(&mut self, full_heap: bool) {
        self.sweep_large_pages(true);
        debug_assert!(self.treadmill.nursery_empty());
        if full_heap {
            self.sweep_large_pages(false);
        }
    }
    // Allow nested-if for this function to make it clear that test_and_mark() is only executed
    // for the outer condition is met.
    #[allow(clippy::collapsible_if)]
    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        let nursery_object = self.is_in_nursery(object);
        if !self.in_nursery_gc || nursery_object {
            // Note that test_and_mark() has side effects
            if self.test_and_mark(object, self.mark_state) {
                let cell = self.get_cell(object);
                self.treadmill.copy(cell, nursery_object);
                self.clear_nursery(object);
                trace.process_node(object);
            }
        }
        object
    }

    fn sweep_large_pages(&mut self, sweep_nursery: bool) {
        // FIXME: borrow checker fighting
        // didn't call self.release_multiple_pages
        // so the compiler knows I'm borrowing two different fields
        if sweep_nursery {
            for cell in self.treadmill.collect_nursery() {
                // println!("- cn {}", cell);
                self.pr.release_pages(get_super_page(cell));
            }
        } else {
            for cell in self.treadmill.collect() {
                // println!("- ts {}", cell);
                self.pr.release_pages(get_super_page(cell));
            }
        }
    }

    /// Allocate an object
    pub fn allocate_pages(&self, tls: VMThread, pages: usize) -> Address {
        self.acquire(tls, pages)
    }

    /// Attempt to mark the object. Return true on success.
    fn test_and_mark(&self, object: ObjectReference, value: u8) -> bool {
        let cell = self.get_cell(object);
        let mut old_value = unsafe { side_metadata::load(Self::MARK_TABLE, cell) } as u8;
        if old_value == value {
            return false;
        }
        while !side_metadata::compare_exchange_atomic(
            Self::MARK_TABLE,
            cell,
            old_value as _,
            value as _,
        ) {
            old_value = unsafe { side_metadata::load(Self::MARK_TABLE, cell) } as u8;
            if old_value == value {
                return false;
            }
        }
        true
    }

    /// Get the mark bit for a given object
    fn test_mark_bit(&self, object: ObjectReference, value: u8) -> bool {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.get_cell(object)) as u8 == value }
    }

    /// Check if a given object is in nursery
    fn is_in_nursery(&self, object: ObjectReference) -> bool {
        unsafe { side_metadata::load(Self::NURSERY_STATE, self.get_cell(object)) == 1 }
    }

    /// Move a given object out of nursery
    fn clear_nursery(&self, object: ObjectReference) {
        side_metadata::store_atomic(Self::NURSERY_STATE, self.get_cell(object), 0)
    }

    /// The the cell of an object
    #[inline(always)]
    fn get_cell(&self, object: ObjectReference) -> Address {
        VM::VMObjectModel::object_start_ref(object)
    }
}

fn get_super_page(cell: Address) -> Address {
    cell.align_down(BYTES_IN_PAGE)
}
