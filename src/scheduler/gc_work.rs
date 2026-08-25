use super::work_bucket::WorkBucketStage;
use super::*;
use crate::vm::*;
use crate::*;
use std::marker::PhantomData;
use std::sync::atomic::Ordering;

/// The kind of a set of roots. Used by LXR to decide how roots should be processed
/// (e.g. reference counting and remembered-set recording).
#[repr(u8)]
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum RootKind {
    /// Ordinary strong roots, e.g. mutator stacks and globals.  These are reference-counted and
    /// marked like any other strong reference.
    Strong,
    /// Roots held by recently JIT-compiled ("young") code-cache entries.  These are recorded into
    /// the remembered set (instead of being reference-counted) so the code cache can be
    /// re-scanned on a later GC.
    YoungCodeCacheRoots,
    /// Roots that hold weak references.  These must not keep their referents alive and are not
    /// reference-counted.
    Weak,
}

impl RootKind {
    /// Whether roots of this kind should be recorded into the remembered set rather than being
    /// processed like normal roots.
    pub fn should_record_remset(&self) -> bool {
        matches!(self, RootKind::YoungCodeCacheRoots)
    }

    /// Whether roots of this kind should skip marking and reference-count decrements.
    pub fn should_skip_mark_and_decs(&self) -> bool {
        matches!(self, RootKind::YoungCodeCacheRoots) || matches!(self, RootKind::Weak)
    }

    /// Whether roots of this kind should skip reference-count decrements.
    pub fn should_skip_decs(&self) -> bool {
        matches!(self, RootKind::YoungCodeCacheRoots) || matches!(self, RootKind::Weak)
    }
}

pub struct ScheduleCollection;

impl<VM: VMBinding> GCWork<VM> for ScheduleCollection {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        // Determine collection kind
        let is_emergency = mmtk.state.set_collection_kind(
            mmtk.get_plan().last_collection_was_exhaustive(),
            mmtk.gc_trigger.policy.can_heap_size_grow(),
        );
        if is_emergency {
            mmtk.get_plan().notify_emergency_collection();
        }
        mmtk.state.stacks_prepared.store(false, Ordering::SeqCst);
        // FIXME: This seems to be a weird place to start counting GC statistics.
        // This is not when we set the status to PauseRequested or InPause. Instead, this is just a random place during transition
        // See https://github.com/mmtk/mmtk-core/issues/1330
        mmtk.stats.start_gc();

        // Let the plan to schedule collection work
        mmtk.get_plan().schedule_collection(worker.scheduler());
    }
}

/// The global GC Preparation Work
/// This work packet invokes prepare() for the plan (which will invoke prepare() for each space), and
/// pushes work packets for preparing mutators and collectors.
/// We should only have one such work packet per GC, before any actual GC work starts.
/// We assume this work packet is the only running work packet that accesses plan, and there should
/// be no other concurrent work packet that accesses plan (read or write). Otherwise, there may
/// be a race condition.
pub struct Prepare<C: GCWorkContext> {
    pub plan: *const C::PlanType,
}

unsafe impl<C: GCWorkContext> Send for Prepare<C> {}

impl<C: GCWorkContext> Prepare<C> {
    pub fn new(plan: *const C::PlanType) -> Self {
        Self { plan }
    }
}

impl<C: GCWorkContext> GCWork<C::VM> for Prepare<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("Prepare Global");
        // We assume this is the only running work packet that accesses plan at the point of execution
        let plan_mut: &mut C::PlanType = unsafe { &mut *(self.plan as *const _ as *mut _) };
        plan_mut.prepare(worker.tls);

        if plan_mut.constraints().needs_prepare_mutator {
            let prepare_mutator_packets = <C::VM as VMBinding>::VMActivePlan::mutators()
                .map(|mutator| Box::new(PrepareMutator::<C::VM>::new(mutator)) as _)
                .collect::<Vec<_>>();
            // Just in case the VM binding is inconsistent about the number of mutators and the actual mutator list.
            debug_assert_eq!(
                prepare_mutator_packets.len(),
                <C::VM as VMBinding>::VMActivePlan::number_of_mutators()
            );
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].bulk_add(prepare_mutator_packets);
        }

        for w in &mmtk.scheduler.worker_group.workers_shared {
            let result = w.designated_work.push(Box::new(PrepareCollector));
            debug_assert!(result.is_ok());
        }
    }
}

/// The mutator GC Preparation Work
pub struct PrepareMutator<VM: VMBinding> {
    // The mutator reference has static lifetime.
    // It is safe because the actual lifetime of this work-packet will not exceed the lifetime of a GC.
    pub mutator: &'static mut Mutator<VM>,
}

impl<VM: VMBinding> PrepareMutator<VM> {
    pub fn new(mutator: &'static mut Mutator<VM>) -> Self {
        Self { mutator }
    }
}

impl<VM: VMBinding> GCWork<VM> for PrepareMutator<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Prepare Mutator");
        self.mutator.prepare(worker.tls);
    }
}

/// The collector GC Preparation Work
#[derive(Default)]
pub struct PrepareCollector;

impl<VM: VMBinding> GCWork<VM> for PrepareCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        trace!("Prepare Collector");
        worker.get_copy_context_mut().prepare();
        mmtk.get_plan().prepare_worker(worker);
    }
}

/// The global GC release Work
/// This work packet invokes release() for the plan (which will invoke release() for each space), and
/// pushes work packets for releasing mutators and collectors.
/// We should only have one such work packet per GC, after all actual GC work ends.
/// We assume this work packet is the only running work packet that accesses plan, and there should
/// be no other concurrent work packet that accesses plan (read or write). Otherwise, there may
/// be a race condition.
pub struct Release<C: GCWorkContext> {
    pub plan: *const C::PlanType,
}

impl<C: GCWorkContext> Release<C> {
    pub fn new(plan: *const C::PlanType) -> Self {
        Self { plan }
    }
}

unsafe impl<C: GCWorkContext> Send for Release<C> {}

impl<C: GCWorkContext + 'static> GCWork<C::VM> for Release<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("Release Global");

        mmtk.gc_trigger.policy.on_gc_release(mmtk);
        // We assume this is the only running work packet that accesses plan at the point of execution

        let plan_mut: &mut C::PlanType = unsafe { &mut *(self.plan as *const _ as *mut _) };
        plan_mut.release(worker.tls);

        let release_mutator_packets = <C::VM as VMBinding>::VMActivePlan::mutators()
            .map(|mutator| Box::new(ReleaseMutator::<C::VM>::new(mutator)) as _)
            .collect::<Vec<_>>();
        // Just in case the VM binding is inconsistent about the number of mutators and the actual mutator list.
        debug_assert_eq!(
            release_mutator_packets.len(),
            <C::VM as VMBinding>::VMActivePlan::number_of_mutators()
        );
        mmtk.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(release_mutator_packets);

        for w in &mmtk.scheduler.worker_group.workers_shared {
            let result = w.designated_work.push(Box::new(ReleaseCollector));
            debug_assert!(result.is_ok());
        }
    }
}

/// The mutator release Work
pub struct ReleaseMutator<VM: VMBinding> {
    // The mutator reference has static lifetime.
    // It is safe because the actual lifetime of this work-packet will not exceed the lifetime of a GC.
    pub mutator: &'static mut Mutator<VM>,
}

impl<VM: VMBinding> ReleaseMutator<VM> {
    pub fn new(mutator: &'static mut Mutator<VM>) -> Self {
        Self { mutator }
    }
}

impl<VM: VMBinding> GCWork<VM> for ReleaseMutator<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Release Mutator");
        self.mutator.release(worker.tls);
    }
}

/// The collector release Work
#[derive(Default)]
pub struct ReleaseCollector;

impl<VM: VMBinding> GCWork<VM> for ReleaseCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Release Collector");
        worker.get_copy_context_mut().release();
    }
}

/// Stop all mutators
///
/// TODO: Smaller work granularity
#[derive(Default)]
pub struct StopMutators<C: GCWorkContext> {
    /// If this is true, we skip creating [`ScanMutatorRoots`] work packets for mutators.
    /// By default, this is false.
    skip_mutator_roots: bool,
    /// If this is true, we skip scanning VM-specific roots.
    /// By default, this is false.
    skip_vm_roots: bool,
    /// Flush mutators once they are stopped. By default this is false. [`ScanMutatorRoots`] will flush mutators.
    flush_mutator: bool,
    phantom: PhantomData<C>,
}

impl<C: GCWorkContext> StopMutators<C> {
    pub fn new() -> Self {
        Self {
            skip_mutator_roots: false,
            skip_vm_roots: false,
            flush_mutator: false,
            phantom: PhantomData,
        }
    }

    pub fn new_with_flush() -> Self {
        let mut me = Self::new();
        me.flush_mutator = true;
        me
    }

    /// Create a `StopMutators` work packet that does not create any root-scanning work packets, and will simply flush mutators.
    pub fn new_no_scan_roots() -> Self {
        Self {
            skip_mutator_roots: true,
            skip_vm_roots: true,
            flush_mutator: true,
            phantom: PhantomData,
        }
    }
}

impl<C: GCWorkContext> GCWork<C::VM> for StopMutators<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("stop_all_mutators start");
        mmtk.state.prepare_for_stack_scanning();
        <C::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls, |mutator| {
            // TODO: The stack scanning work won't start immediately, as the `Prepare` bucket is not opened yet (the bucket is opened in notify_mutators_paused).
            // Should we push to Unconstrained instead?

            if self.flush_mutator {
                mutator.flush();
            }
            if !self.skip_mutator_roots {
                mmtk.scheduler.work_buckets[mmtk.get_plan().root_scanning_stage()]
                    .add(ScanMutatorRoots::<C>(mutator));
            }
        });
        trace!("stop_all_mutators end");
        // All mutators have just stopped: this is the end of the "time-to-yield" window that
        // started when the pause was successfully requested (see `GCTrigger::request()`), and
        // the start of the "pause time" window that ends when mutators are resumed (see
        // `GCWorkScheduler::on_gc_finished`).
        mmtk.stats.record_time_to_yield(mmtk.state.take_time_to_yield());
        mmtk.state.record_pause_start_time();
        // This also tells the GC trigger whether a new GC cycle has started (see `Plan::gc_pause_start`).
        mmtk.get_plan().on_pause_start(mmtk);
        mmtk.scheduler.notify_mutators_paused(mmtk);
        // Tell GC trigger that the pause started.
        mmtk.gc_trigger.policy.on_pause_start(mmtk);
        if !self.skip_vm_roots {
            let factory = C::make_roots_work_factory(mmtk);
            <C::VM as VMBinding>::VMScanning::scan_vm_specific_roots(worker.tls, factory);
        }
    }
}

pub struct ScanMutatorRoots<C: GCWorkContext>(pub &'static mut Mutator<C::VM>);

impl<C: GCWorkContext> GCWork<C::VM> for ScanMutatorRoots<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("ScanMutatorRoots for mutator {:?}", self.0.get_tls());
        let mutators = <C::VM as VMBinding>::VMActivePlan::number_of_mutators();
        let factory = C::make_roots_work_factory(mmtk);
        <C::VM as VMBinding>::VMScanning::scan_roots_in_mutator_thread(
            worker.tls,
            unsafe { &mut *(self.0 as *mut _) },
            factory,
        );
        self.0.flush();

        if mmtk.state.inform_stack_scanned(mutators) {
            <C::VM as VMBinding>::VMScanning::notify_initial_thread_scan_complete(
                false, worker.tls,
            );
        }
    }
}

#[derive(Default)]
pub struct ScanVMSpecificRoots<C: GCWorkContext>(PhantomData<C>);

impl<C: GCWorkContext> ScanVMSpecificRoots<C> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<C: GCWorkContext> GCWork<C::VM> for ScanVMSpecificRoots<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("ScanStaticRoots");
        let factory = C::make_roots_work_factory(mmtk);
        <C::VM as VMBinding>::VMScanning::scan_vm_specific_roots(worker.tls, factory);
    }
}
