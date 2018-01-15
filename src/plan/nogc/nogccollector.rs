use super::NoGCTraceLocal;
use super::super::ParallelCollectorGroup;
use super::super::ParallelCollector;
use super::super::CollectorContext;
use super::super::TraceLocal;
use super::super::Phase;

use ::util::{Address, ObjectReference};

pub struct NoGCCollector<'a> {
    pub id: usize,
    trace: NoGCTraceLocal,

    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'a ParallelCollectorGroup<NoGCCollector<'a>>>,
}

impl<'a> CollectorContext for NoGCCollector<'a> {
    fn new() -> Self {
        NoGCCollector {
            id: 0,
            trace: NoGCTraceLocal::new(),

            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

    fn init(&mut self, id: usize) {
        self.id = id;
    }

    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: usize) -> Address {
        unimplemented!();
    }

    fn run(&mut self, thread_id: usize) {
        self.park();
        self.collect();
    }

    fn collection_phase(&mut self, phase: Phase, primary: bool) {
        unimplemented!();
    }
}

impl<'a> ParallelCollector for NoGCCollector<'a> {
    fn park(&mut self) {
        self.group.unwrap().park(self);
    }
    fn collect(&self) {
        unimplemented!();
    }
    fn get_current_trace<T: TraceLocal>(&self) -> T {
        unimplemented!()
    }
    fn parallel_worker_count(&self) -> usize {
        self.group.unwrap().active_worker_count()
    }
    fn parallel_worker_ordinal(&self) -> usize {
        self.worker_ordinal
    }
    fn rendezvous(&self) -> usize {
        self.group.unwrap().rendezvous()
    }

    fn get_last_trigger_count(&self) -> usize {
        self.last_trigger_count
    }
    fn set_last_trigger_count(&mut self, val: usize) {
        self.last_trigger_count = val;
    }
    fn increment_last_trigger_count(&mut self) {
        self.last_trigger_count += 1;
    }

    fn set_group(&mut self, group: *const ParallelCollectorGroup<Self>) {
        self.group = Some ( unsafe {&*group} );
    }
    fn set_worker_ordinal(&mut self, ordinal: usize) {
        self.worker_ordinal = ordinal;
    }
}