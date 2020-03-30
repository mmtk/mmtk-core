use crate::util::alloc::linear_scan::LinearScan;
use crate::util::ObjectReference;

use crate::vm::ObjectModel;
use crate::vm::VMBinding;

pub struct DumpLinearScan {}

impl LinearScan for DumpLinearScan {
    fn scan<VM: VMBinding>(&self, object: ObjectReference) {
        println!(
            "[{}], SIZE = {}",
            object.to_address(),
            VM::VMObjectModel::get_current_size(object)
        );
    }
}
