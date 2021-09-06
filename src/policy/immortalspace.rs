use atomic::Ordering;

use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::address::Address;
use crate::util::heap::{MonotonePageResource, PageResource, VMRequest};

use crate::util::constants::CARD_META_PAGES_PER_REGION;
use crate::util::metadata::{compare_exchange_metadata, load_metadata, store_metadata};
use crate::util::{metadata, ObjectReference};

use crate::plan::TransitiveClosure;

use crate::plan::PlanConstraints;
use crate::policy::space::SpaceOptions;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::vm::{ObjectModel, VMBinding};

/// This type implements a simple immortal collection
/// policy. Under this policy all that is required is for the
/// "collector" to propagate marks in a liveness trace.  It does not
/// actually collect.
pub struct ImmortalSpace<VM: VMBinding> {
    mark_state: usize,
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
}

const GC_MARK_BIT_MASK: usize = 1;
const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

impl<VM: VMBinding> SFT for ImmortalSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, _object: ObjectReference) -> bool {
        true
    }
    #[inline(always)]
    fn is_reachable(&self, object: ObjectReference) -> bool {
        let old_value = load_metadata::<VM>(
            &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        );
        old_value == self.mark_state
    }
    fn is_movable(&self) -> bool {
        false
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        let old_value = load_metadata::<VM>(
            &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        );
        let new_value = (old_value & GC_MARK_BIT_MASK) | self.mark_state;
        store_metadata::<VM>(
            &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            object,
            new_value,
            None,
            Some(Ordering::SeqCst),
        );

        if self.common.needs_log_bit {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(object, Ordering::SeqCst);
        }
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::set_alloc_bit(object);
    }
}

impl<VM: VMBinding> Space<VM> for ImmortalSpace<VM> {
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
        &self.common
    }

    fn init(&mut self, _vm_map: &'static VMMap) {
        self.common().init(self.as_space());
    }
    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> ImmortalSpace<VM> {
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
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: true,
                needs_log_bit: constraints.needs_log_bit,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: metadata::extract_side_metadata(&[
                        *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                    ]),
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        ImmortalSpace {
            mark_state: 0,
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
            common,
        }
    }

    fn test_and_mark(object: ObjectReference, value: usize) -> bool {
        loop {
            let old_value = load_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                None,
                Some(Ordering::SeqCst),
            );
            if old_value == value {
                return false;
            }

            if compare_exchange_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                old_value,
                old_value ^ GC_MARK_BIT_MASK,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                break;
            }
        }
        true
    }

    pub fn prepare(&mut self) {
        self.mark_state = GC_MARK_BIT_MASK - self.mark_state;
    }

    pub fn release(&mut self) {}

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        #[cfg(feature = "global_alloc_bit")]
        debug_assert!(
            crate::util::alloc_bit::is_alloced(object),
            "{:x}: alloc bit not set",
            object
        );
        if ImmortalSpace::<VM>::test_and_mark(object, self.mark_state) {
            trace.process_node(object);
        }
        object
    }
}
