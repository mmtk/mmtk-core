use crate::util::ObjectReference;
use crate::util::Address;
use crate::vm::VMBinding;
use crate::vm::ObjectModel;
use crate::util::address::ByteSize;

/// Callbacks during a linear scan are dispatched through
/// an implementation of this object.
pub trait LinearScanCallback {
    /// Returns the object size in bytes. Linear scan will skip this many bytes.
    fn on_object(&mut self, object: ObjectReference) -> ByteSize;
    fn on_page(&mut self, _previous_page: Address) {}
}

pub struct DumpObject<VM: VMBinding>(std::marker::PhantomData<VM>);

impl<VM: VMBinding> LinearScanCallback for DumpObject<VM> {
    fn on_object(&mut self, object: ObjectReference) -> ByteSize {
        let size = VM::VMObjectModel::get_current_size(object);
        println!(
            "[{}], SIZE = {}",
            object.to_address(),
            size
        );
        size
    }
}
