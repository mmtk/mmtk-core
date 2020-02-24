use ::util::ObjectReference;
use vm::VMBinding;

pub trait LinearScan{
    fn scan<VM: VMBinding>(&self, object: ObjectReference);
}