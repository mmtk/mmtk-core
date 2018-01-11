use super::CollectorContext;
use super::Phase;
use super::TraceLocal;

pub trait ParallelCollector: CollectorContext {
    fn collect(&self);
    fn collection_phase(&self, phase_id: Phase, primary: bool);
    fn get_current_trace<T: TraceLocal>() -> T;
    fn parallel_worker_count(&self) -> usize;
    fn parallel_worker_ordinal(&self) -> usize;
    fn rendezvous(&self) -> usize;

    fn get_last_trigger_count(&self) -> usize;
    fn set_last_trigger_count(&self, val: usize);
    fn increment_last_trigger_count(&self) {
        self.set_last_trigger_count(self.get_last_trigger_count() + 1);
    }
}