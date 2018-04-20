use ::util::ObjectReference;

pub trait LinearScan{
    fn scan(object: ObjectReference);
}