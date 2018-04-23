use ::util::ObjectReference;

pub trait LinearScan{
    fn scan(&self, object: ObjectReference);
}