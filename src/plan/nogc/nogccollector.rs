use super::NoGCTraceLocal;
use super::super::ParallelCollectorGroup;
use super::super::ParallelCollector;
use super::super::CollectorContext;
use super::super::TraceLocal;
use super::super::Phase;
use super::super::Allocator;
use plan::nogc::SelectedPlan;

use std::process;
use libc::c_void;

use ::util::{Address, ObjectReference};
use util::OpaquePointer;

pub struct NoGCCollector {
    pub tls: OpaquePointer,
    trace: NoGCTraceLocal,

    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'static ParallelCollectorGroup<NoGCCollector>>,
}

impl<'a> CollectorContext for NoGCCollector {
    fn new(_: &'static SelectedPlan) -> Self {
        NoGCCollector {
            tls: OpaquePointer::UNINITIALIZED,
            trace: NoGCTraceLocal::new(),

            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

    fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
    }

    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: Allocator) -> Address {
        unimplemented!();
    }

    fn run(&mut self, tls: OpaquePointer) {
        loop {
            self.park();
            self.collect();
        }
    }

    fn collection_phase(&mut self, tls: OpaquePointer, phase: &Phase, primary: bool) {
        println!("GC triggered in NoGC plan");
        process::exit(128);
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }
}

impl ParallelCollector for NoGCCollector {
    type T = NoGCTraceLocal;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        println!("GC triggered in NoGC plan");
        process::exit(128);
    }

    fn get_current_trace(&mut self) -> &mut NoGCTraceLocal {
        &mut self.trace
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