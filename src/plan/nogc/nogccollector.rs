use super::NoGCTraceLocal;
use super::super::ParallelCollectorGroup;
use super::super::ParallelCollector;
use super::super::CollectorContext;
use super::super::Phase;
use super::super::Allocator;

use std::process;

use ::util::{Address, ObjectReference};
use util::OpaquePointer;
use mmtk::MMTK;
use vm::VMBinding;

pub struct NoGCCollector<VM: VMBinding> {
    pub tls: OpaquePointer,
    trace: NoGCTraceLocal<VM>,

    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'static ParallelCollectorGroup<VM, NoGCCollector<VM>>>,
}

impl<VM: VMBinding> CollectorContext<VM> for NoGCCollector<VM> {
    fn new(_: &'static MMTK<VM>) -> Self {
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

    fn alloc_copy(&mut self, _original: ObjectReference, _bytes: usize, _align: usize, _offset: isize, _allocator: Allocator) -> Address {
        unreachable!()
    }

    fn run(&mut self, _tls: OpaquePointer) {
        loop {
            self.park();
            self.collect();
        }
    }

    fn collection_phase(&mut self, _tls: OpaquePointer, _phase: &Phase, _primary: bool) {
        println!("GC triggered in NoGC plan");
        process::exit(128);
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }
}

impl<VM: VMBinding> ParallelCollector<VM> for NoGCCollector<VM> {
    type T = NoGCTraceLocal<VM>;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        println!("GC triggered in NoGC plan");
        process::exit(128);
    }

    fn get_current_trace(&mut self) -> &mut NoGCTraceLocal<VM> {
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

    // See ParallelCollector.set_group()
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn set_group(&mut self, group: *const ParallelCollectorGroup<VM, Self>) {
        self.group = Some ( unsafe {&*group} );
    }

    fn set_worker_ordinal(&mut self, ordinal: usize) {
        self.worker_ordinal = ordinal;
    }
}