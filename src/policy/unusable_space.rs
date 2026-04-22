use crate::plan::VectorObjectQueue;
use crate::policy::sft::{GCWorkerMutRef, SFT};
use crate::policy::space::{CommonSpace, Space};
use crate::util::heap::PageResource;
use crate::util::object_enum::ObjectEnumerator;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

#[derive(Debug)]
pub(crate) struct UnusableSpace;

pub(crate) static UNUSABLE_SPACE: UnusableSpace = UnusableSpace;

pub(crate) fn unusable_space<VM: VMBinding>() -> &'static dyn Space<VM> {
    &UNUSABLE_SPACE
}

#[cold]
#[track_caller]
fn panic_unusable_space(method: &str) -> ! {
    panic!("Called {method} on UnusableSpace. The allocator is not configured for this plan.")
}

impl SFT for UnusableSpace {
    fn name(&self) -> &'static str {
        panic_unusable_space("SFT::name")
    }

    fn get_forwarded_object(&self, _object: ObjectReference) -> Option<ObjectReference> {
        panic_unusable_space("SFT::get_forwarded_object")
    }

    fn is_live(&self, _object: ObjectReference) -> bool {
        panic_unusable_space("SFT::is_live")
    }

    fn is_reachable(&self, _object: ObjectReference) -> bool {
        panic_unusable_space("SFT::is_reachable")
    }

    #[cfg(feature = "object_pinning")]
    fn pin_object(&self, _object: ObjectReference) -> bool {
        panic_unusable_space("SFT::pin_object")
    }

    #[cfg(feature = "object_pinning")]
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        panic_unusable_space("SFT::unpin_object")
    }

    #[cfg(feature = "object_pinning")]
    fn is_object_pinned(&self, _object: ObjectReference) -> bool {
        panic_unusable_space("SFT::is_object_pinned")
    }

    fn is_movable(&self) -> bool {
        panic_unusable_space("SFT::is_movable")
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        panic_unusable_space("SFT::is_sane")
    }

    fn is_in_space(&self, _object: ObjectReference) -> bool {
        panic_unusable_space("SFT::is_in_space")
    }

    #[cfg(feature = "vo_bit")]
    fn is_mmtk_object(&self, _addr: Address) -> Option<ObjectReference> {
        panic_unusable_space("SFT::is_mmtk_object")
    }

    #[cfg(feature = "vo_bit")]
    fn find_object_from_internal_pointer(
        &self,
        _ptr: Address,
        _max_search_bytes: usize,
    ) -> Option<ObjectReference> {
        panic_unusable_space("SFT::find_object_from_internal_pointer")
    }

    fn initialize_object_metadata(&self, _object: ObjectReference) {
        panic_unusable_space("SFT::initialize_object_metadata")
    }

    fn sft_trace_object(
        &self,
        _queue: &mut VectorObjectQueue,
        _object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        panic_unusable_space("SFT::sft_trace_object")
    }

    fn debug_print_object_info(&self, _object: ObjectReference) {
        panic_unusable_space("SFT::debug_print_object_info")
    }
}

impl<VM: VMBinding> Space<VM> for UnusableSpace {
    fn as_space(&self) -> &dyn Space<VM> {
        panic_unusable_space("Space::as_space")
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        panic_unusable_space("Space::as_sft")
    }

    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        panic_unusable_space("Space::get_page_resource")
    }

    fn maybe_get_page_resource_mut(&mut self) -> Option<&mut dyn PageResource<VM>> {
        panic_unusable_space("Space::maybe_get_page_resource_mut")
    }

    fn initialize_sft(&self, _sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        panic_unusable_space("Space::initialize_sft")
    }

    fn common(&self) -> &CommonSpace<VM> {
        panic_unusable_space("Space::common")
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic_unusable_space("Space::release_multiple_pages")
    }

    fn enumerate_objects(&self, _enumerator: &mut dyn ObjectEnumerator) {
        panic_unusable_space("Space::enumerate_objects")
    }

    fn clear_side_log_bits(&self) {
        panic_unusable_space("Space::clear_side_log_bits")
    }

    fn set_side_log_bits(&self) {
        panic_unusable_space("Space::set_side_log_bits")
    }
}
