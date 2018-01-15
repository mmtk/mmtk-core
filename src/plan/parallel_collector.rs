use super::ParallelCollectorGroup;
use super::CollectorContext;
use super::TraceLocal;

pub trait ParallelCollector: CollectorContext + Sized {
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

    fn set_group(&mut self, group: *const ParallelCollectorGroup<Self>);
    fn set_worker_ordinal(&mut self, ordinal: usize);
}