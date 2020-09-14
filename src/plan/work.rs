use super::worker::*;
use super::scheduler::*;
use crate::vm::VMBinding;
use crate::mmtk::MMTK;
use crate::plan::Plan;
use std::sync::{Arc, Barrier};
use std::marker::PhantomData;
use crate::vm::*;
use crate::util::{ObjectReference, Address};
use crate::plan::TransitiveClosure;
use std::mem;



pub trait Work: 'static + Send + Sync {
    fn requires_stop_the_world(&self) -> bool { false }
    fn do_work(&mut self, worker: &Worker, scheduler: &'static Scheduler);
}

impl PartialEq for Box<dyn Work> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() as *const dyn Work == other.as_ref() as *const dyn Work
    }
}

impl Eq for Box<dyn Work> {}




/// GC Preparation Work (include updating global states)
pub struct Prepare<P: Plan> {
    pub plan: &'static P,
}

unsafe impl <P: Plan> Sync for Prepare<P> {}

impl <P: Plan> Prepare<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl <P: Plan> Work for Prepare<P> {
    fn do_work(&mut self, worker: &Worker, scheduler: &'static Scheduler) {
        println!("Prepare");
    }
}

/// Stop all mutators
///
/// Schedule a `ScanStackRoots` immediately after a mutator is paused
///
/// TODO: Smaller work granularity
#[derive(Default)]
pub struct StopMutators<P: Plan>(PhantomData<P>);

impl <P: Plan> StopMutators<P> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl <P: Plan> Work for StopMutators<P> {
    fn do_work(&mut self, worker: &Worker, scheduler: &'static Scheduler) {
        println!("stop_all_mutators start");
        <P::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls);
        println!("stop_all_mutators end");
        scheduler.mutators_stopped();
        scheduler.add_with_highest_priority(ScanStackRoots::<TestProcessEdges<P>>::new());
    }
}

#[derive(Default)]
pub struct ScanStackRoots<Edges: ProcessEdges>(PhantomData<(Edges)>);

impl <Edges: ProcessEdges> ScanStackRoots<Edges> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl <Edges: ProcessEdges> Work for ScanStackRoots<Edges> {
    fn do_work(&mut self, worker: &Worker, _scheduler: &'static Scheduler) {
        <<Edges::Plan as Plan>::VM as VMBinding>::VMScanning::scan_thread_roots::<Edges>(worker.tls);
    }
}

/// Scan & update a list of object slots
pub trait ProcessEdges: Work {
    type Plan: Plan;
    const CAPACITY: usize = 512;
    fn new(edges: Vec<Address>, roots: bool) -> Self;
}

#[derive(Default)]
struct TestProcessEdges<P: Plan>(Vec<Address>, PhantomData<P>);

impl <P: Plan> ProcessEdges for TestProcessEdges<P> {
    type Plan = P;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self(edges, PhantomData)
    }
}
impl <P: Plan> Work for TestProcessEdges<P> {
    fn requires_stop_the_world(&self) -> bool { true }
    fn do_work(&mut self, worker: &Worker, _scheduler: &'static Scheduler) {
        println!("TestProcessEdges::do_work");
    }
}

/// Scan & update a list of object slots
pub struct ScanObjects<Edges: ProcessEdges> {
    buffer: Vec<ObjectReference>,
    concurrent: bool,
    phantom: PhantomData<Edges>,
}

impl <Edges: ProcessEdges> ScanObjects<Edges> {
    pub fn new(buffer: Vec<ObjectReference>, concurrent: bool) -> Self {
        Self { buffer, concurrent, phantom: PhantomData }
    }
}

impl <Edges: ProcessEdges> Work for ScanObjects<Edges> {
    fn requires_stop_the_world(&self) -> bool { !self.concurrent }
    fn do_work(&mut self, worker: &Worker, _scheduler: &'static Scheduler) {
        println!("ScanObjects");
        <<Edges::Plan as Plan>::VM as VMBinding>::VMScanning::scan_objects::<Edges>(&self.buffer);
    }
}

// pub struct TraceStrongRefs<Trace: TraceObjects>(pub &'static MMTK<VM>);

// impl <Trace: TraceObjects> Work for TraceStrongRefs<Trace> {
//     fn do_work(&mut self, worker: &Worker, scheduler: &Scheduler) {
//         scheduler.stw_bucket.add(box ScanStackRoots);
//         scheduler.stw_bucket.add(box ScanGlobalRoots);
//     }
// }

// /// Stop all mutators
// ///
// /// Schedule a `ScanStackRoots` immediately after a mutator is paused
// pub struct StopMutators<VM: VMBinding>(pub &'static MMTK<VM>);

// impl <VM: VMBinding> Work for StopMutators<VM> {
//     fn do_work(&mut self, worker: &Worker, scheduler: &Scheduler) {
//         VM::VMCollection::stop_all_mutators(worker.tls);
//         self.0.plan.base().control_collector_context.clear_request();
//         scheduler.mutators_stopped();
//         scheduler.stw_bucket.add(box ScanStackRoots);
//         scheduler.stw_bucket.add(box ScanGlobalRoots);
//     }
// }

// #[derive(Default)]
// pub struct ScanStackRoots<Trace: TraceObjects>(PhantomData<(VM, Trace)>);

// impl <Trace: TraceObjects> Work for ScanStackRoots<Trace> {
//     fn do_work(&mut self, scheduler: &Scheduler) {
//         VM::VMScanning::compute_thread_roots(&mut self.trace, self.tls);
//     }
// }

// #[derive(Default)]
// pub struct ScanGlobalRoots<Trace: TraceObjects>(PhantomData<Trace>);

// impl <Trace: TraceObjects> Work for ScanGlobalRoots<Trace> {
//     fn requires_stop_the_world(&self) -> bool { true }
//     fn do_work(&mut self, scheduler: &Scheduler) {
//         Trace::VM::VMScanning::compute_global_roots(&mut self.trace, self.tls);
//         Trace::VM::VMScanning::compute_static_roots(&mut self.trace, self.tls);
//         // if super::global::SCAN_BOOT_IMAGE {
//         //     VM::VMScanning::compute_bootimage_roots(&mut self.trace, self.tls);
//         // }
//     }
// }

// impl <T: TraceObjects> TraceLocal for ScanGlobalRoots<T> {
//     fn report_delayed_root_edge(&mut self, slot: Address) {

//     }

//     fn process_roots(&mut self) { unreachable!() }
//     fn process_root_edge(&mut self, slot: Address, untraced: bool) { unreachable!() }
//     fn trace_object(&mut self, object: ObjectReference) -> ObjectReference { unreachable!() }
//     fn complete_trace(&mut self) { unreachable!() }
//     fn release(&mut self) { unreachable!() }
//     fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, root: bool) { unreachable!() }
//     fn overwrite_reference_during_trace(&self) -> bool { unreachable!() }
//     fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool { unreachable!() }
//     fn get_forwarded_reference(&mut self, object: ObjectReference) -> ObjectReference { unreachable!() }
//     fn get_forwarded_referent(&mut self, object: ObjectReference) -> ObjectReference { unreachable!() }
//     fn retain_referent(&mut self, object: ObjectReference) -> ObjectReference { unreachable!() }
// }


// impl <T: TraceObjects> TransitiveClosure for ScanGlobalRoots<T> {
//     fn process_edge(&mut self, src: ObjectReference, slot: Address) {
//         unreachable!()
//     }
//     fn process_node(&mut self, object: ObjectReference) {
//         unreachable!()
//     }
// }

// pub struct ScanObjects<E: ProcessEdges> {
//     pub buffer: Vec<ObjectReference>,
//     edges: Vec<(Option<ObjectReference>, Address)>,
//     phantom: PhantomData<E>,
//     scheduler: Option<&'static Scheduler>,
// }

// impl <E: ProcessEdges> ScanObjects<E> {
//     pub fn new(buffer: Vec<ObjectReference>) -> Self {
//         Self { buffer, edges: vec![], phantom: PhantomData }
//     }
// }

// impl <E: ProcessEdges> TransitiveClosure for ScanObjects<E> {
//     fn process_edge(&mut self, src: ObjectReference, slot: Address) {
//         self.edges.push((Some(src), slot));
//         if self.edges.len() > E::BUFFER_LENGTH {
//             // Create a new `ProcessEdges` work
//             let mut empty_edges = vec![];
//             mem::swap(&mut empty_edges, &mut self.edges);
//             self.scheduler.unwrap().add_with_highest_priority(box E::new(empty_edges));
//         }
//     }
//     fn process_node(&mut self, object: ObjectReference) {
//         unreachable!()
//     }
// }

// impl <E: ProcessEdges> Work for ScanObjects<E> {
//     fn requires_stop_the_world(&self) -> bool { true }
//     fn do_work(&mut self, worker: &Worker, scheduler: &'static Scheduler) {
//         self.scheduler = Some(scheduler);
//         for object in self.0 {
//             <E::VM as VMBinding>::VMScanning::scan_object(self, *object, worker.tls);
//         }
//     }
// }

// pub trait ProcessEdges: Work {
//     type VM: VMBinding;
//     const OVERWRITE_REFERENCE: bool = true;
//     const BUFFER_LENGTH: usize = 512;
//     fn new(edges: Vec<(Option<ObjectReference>, Address)>) -> Self;
//     /// Get the list of objects to scan
//     fn edges(&self) -> &[(Option<ObjectReference>, Address)];
//     /// Create a new `TraceObjects` with a new buffer
//     fn clone_with_buffer(&self, buf: Vec<ObjectReference>) -> Self;
//     /// Add the object to a queue of unscanned nodes.
//     ///
//     /// If the queue is full, create a new `TraceObjects` work and send to the scheduler
//     fn process_node(&mut self, object: ObjectReference);
//     /// Scan an edge
//     fn process_edge(&mut self, slot: Address) {
//         let mut object = unsafe { slot.load::<ObjectReference>() };
//         object = self.trace_object(object);
//         if Self::OVERWRITE_REFERENCE {
//             unsafe { slot.store(object) };
//         }
//     }
//     /// Trace an object
//     fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;
// }

// impl <T: ProcessEdges> Work for T {
//     fn requires_stop_the_world(&self) -> bool { true }
//     fn do_work(&mut self, worker: &Worker, scheduler: &'static Scheduler) {
//         for slot in self.edges() {
//             self.process_edge(slot);
//         }
//     }
// }

// pub struct Release;

// impl Work for Release {
//     fn requires_stop_the_world(&self) -> bool { true }
//     fn do_work(&mut self, scheduler: &Scheduler) {}
// }