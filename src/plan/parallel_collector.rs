use super::CollectorContext;
use super::Phase;
use super::TraceLocal;

pub trait ParallelCollector: CollectorContext {
    fn collect();
    fn collection_phase(phase_id: Phase, primary: bool);
    fn get_current_trace<T: TraceLocal>() -> T;
    fn parallel_worker_count() -> usize;
    fn parallel_worker_ordinal() -> usize;
    fn rendezvous() -> usize;
}