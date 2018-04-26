use ::util::alloc::linear_scan::LinearScan;
use ::util::{Address, ObjectReference};

use ::vm::{ObjectModel, VMObjectModel};

pub struct DumpLinearScan {}

impl LinearScan for DumpLinearScan {
    fn scan(&self, object: ObjectReference) {
        println!("[{}], SIZE = {}",
                 object.to_address(),
                 VMObjectModel::get_current_size(object)
        );
    }
}
