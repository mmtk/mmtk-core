use crate::util::ObjectReference;
use crate::vm::VMBinding;

pub trait LinearScan{
    fn scan<VM: VMBinding>(&self, object: ObjectReference);
}