use ::util::alloc::linear_scan::LinearScan;
use ::util::{Address, ObjectReference};

use ::vm::ObjectModel;
use vm::VMBinding;

pub struct DumpLinearScan {}

impl LinearScan for DumpLinearScan {
    fn scan<VM: VMBinding>(&self, object: ObjectReference) {
        println!("[{}], SIZE = {}",
                 object.to_address(),
                 VM::VMObjectModel::get_current_size(object)
        );
    }
}
