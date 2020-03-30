use super::ParallelCollectorGroup;
use super::CollectorContext;
use super::TraceLocal;
use vm::VMBinding;

pub trait ParallelCollector<VM: VMBinding>: CollectorContext<VM> + Sized {
    type T: TraceLocal;

    fn park(&mut self);
    fn collect(&self);
    fn get_current_trace(&mut self) -> &mut Self::T;
    fn parallel_worker_count(&self) -> usize;
    fn parallel_worker_ordinal(&self) -> usize;
    fn rendezvous(&self) -> usize;

    fn get_last_trigger_count(&self) -> usize;
    fn set_last_trigger_count(&mut self, val: usize);
    fn increment_last_trigger_count(&mut self);

    // The implementation of this function probably will dereference a pointer argument, but there
    // is no way to guarantee the pointer is valid. Thus the function should be marked as unsafe.
    // However, we may be able to remove the use of the raw pointer here. It was unnecessary to use
    // raw pointers here merely to allow circular dependency. We should fix this instead.
    // FIXME: remove the use of raw pointer here.
    fn set_group(&mut self, group: *const ParallelCollectorGroup<VM, Self>);
    fn set_worker_ordinal(&mut self, ordinal: usize);
}