use super::gc_work::ImmixGCWorkContext;
use super::mutator::ALLOCATOR_MAPPING;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::MutatorContext;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::{TRACE_KIND_DEFRAG, TRACE_KIND_FAST, TRACE_KIND_IMMOVABLE};
use crate::policy::space::Space;
use crate::scheduler::gc_work::PlanProcessEdges;
use crate::scheduler::gc_work::ScanVMImmovableRoots;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::HeapMeta;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::options::Options;
use crate::vm::ActivePlan;
use crate::vm::Collection;
use crate::vm::Scanning;
use crate::vm::VMBinding;
use crate::MMTK;
use crate::{policy::immix::ImmixSpace, util::opaque_pointer::VMWorkerThread};
use std::marker::PhantomData;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use atomic::Ordering;
use enum_map::EnumMap;

use mmtk_macros::PlanTraceObject;

#[derive(PlanTraceObject)]
pub struct Immix<VM: VMBinding> {
    #[post_scan]
    #[trace(CopySemantics::DefaultCopy)]
    pub immix_space: ImmixSpace<VM>,
    #[fallback_trace]
    pub common: CommonPlan<VM>,
    last_gc_was_defrag: AtomicBool,
}

pub const IMMIX_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: crate::policy::immix::DEFRAG,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    /// Max immix object size is half of a block.
    max_non_los_default_alloc_bytes: crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for Immix<VM> {
    type VM = VM;

    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        ImmixSpace::<VM>::is_last_gc_exhaustive(self.last_gc_was_defrag.load(Ordering::Relaxed))
    }

    fn constraints(&self) -> &'static PlanConstraints {
        &IMMIX_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::Immix(0), &self.immix_space)],
            constraints: &IMMIX_CONSTRAINTS,
        }
    }

    fn get_spaces(&self) -> Vec<&dyn Space<Self::VM>> {
        let mut ret = self.common.get_spaces();
        ret.push(&self.immix_space);
        ret
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        let in_defrag = self.immix_space.decide_whether_to_defrag(
            self.is_emergency_collection(),
            true,
            self.base().cur_collection_attempts.load(Ordering::SeqCst),
            self.base().is_user_triggered_collection(),
            *self.base().options.full_heap_system_gc,
        );

        // The blocks are not identical, clippy is wrong. Probably it does not recognize the constant type parameter.
        #[allow(clippy::if_same_then_else)]
        if in_defrag {
            schedule_stop_mutator_scan_immobile_roots::<
                VM,
                ImmixGCWorkContext<VM, TRACE_KIND_IMMOVABLE>,
            >(scheduler, self);
            schedule_remaining_work::<VM, ImmixGCWorkContext<VM, TRACE_KIND_DEFRAG>>(
                scheduler, self,
            );
        } else {
            scheduler.schedule_common_work::<ImmixGCWorkContext<VM, TRACE_KIND_FAST>>(self);
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.immix_space.prepare(true);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        // release the collected region
        self.last_gc_was_defrag
            .store(self.immix_space.release(true), Ordering::Relaxed);
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.immix_space.defrag_headroom_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.immix_space.reserved_pages() + self.common.get_used_pages()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

/// Stop all mutators and scan immovable roots
///
/// Schedule a `ScanVMImmovableRoots` immediately after a mutator is paused
///
/// TODO: Smaller work granularity
#[derive(Default)]
pub struct StopMutatorScanImmovable<ScanEdges: ProcessEdgesWork>(PhantomData<ScanEdges>);

impl<ScanEdges: ProcessEdgesWork> StopMutatorScanImmovable<ScanEdges> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork> CoordinatorWork<E::VM> for StopMutatorScanImmovable<E> {}

impl<E: ProcessEdgesWork> GCWork<E::VM> for StopMutatorScanImmovable<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // If the VM requires that only the coordinator thread can stop the world,
        // we delegate the work to the coordinator.
        if <E::VM as VMBinding>::VMCollection::COORDINATOR_ONLY_STW && !worker.is_coordinator() {
            mmtk.scheduler
                .add_coordinator_work(StopMutatorScanImmovable::<E>::new(), worker);
            return;
        }

        trace!("stop_all_mutators start");
        mmtk.plan.base().prepare_for_stack_scanning();
        <E::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls, |mutator| {
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].add(ScanStackRoot::<E>(mutator));
        });
        trace!("stop_all_mutators end");
        mmtk.scheduler.notify_mutators_paused(mmtk);
        if <E::VM as VMBinding>::VMScanning::SCAN_MUTATORS_IN_SAFEPOINT {
            // Prepare mutators if necessary
            // FIXME: This test is probably redundant. JikesRVM requires to call `prepare_mutator` once after mutators are paused
            if !mmtk.plan.base().stacks_prepared() {
                for mutator in <E::VM as VMBinding>::VMActivePlan::mutators() {
                    <E::VM as VMBinding>::VMCollection::prepare_mutator(
                        worker.tls,
                        mutator.get_tls(),
                        mutator,
                    );
                }
            }
            // Scan immovable roots`
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].add(ScanVMImmovableRoots::<
                PlanProcessEdges<E::VM, Immix<E::VM>, TRACE_KIND_IMMOVABLE>,
            >::new());
        }
    }
}

fn schedule_stop_mutator_scan_immobile_roots<VM: VMBinding, C: GCWorkContext<VM = VM> + 'static>(
    scheduler: &GCWorkScheduler<VM>,
    _plan: &'static C::PlanType,
) {
    // Stop & scan mutators (mutator scanning can happen before STW)
    scheduler.work_buckets[WorkBucketStage::Unconstrained]
        .add(StopMutatorScanImmovable::<C::ProcessEdgesWorkType>::new());
}

fn schedule_remaining_work<VM: VMBinding, C: GCWorkContext<VM = VM> + 'static>(
    scheduler: &GCWorkScheduler<VM>,
    plan: &'static C::PlanType,
) {
    use crate::scheduler::gc_work::*;
    // Scan mutators (mutator scanning can happen before STW)
    scheduler.work_buckets[WorkBucketStage::Prepare]
        .add(ScanMutators::<C::ProcessEdgesWorkType>::new());

    // Prepare global/collectors/mutators
    scheduler.work_buckets[WorkBucketStage::Prepare].add(Prepare::<C>::new(plan));

    // Release global/collectors/mutators
    scheduler.work_buckets[WorkBucketStage::Release].add(Release::<C>::new(plan));

    // Analysis GC work
    #[cfg(feature = "analysis")]
    {
        use crate::util::analysis::GcHookWork;
        scheduler.work_buckets[WorkBucketStage::Unconstrained].add(GcHookWork);
    }

    // Sanity
    #[cfg(feature = "sanity")]
    {
        use crate::util::sanity::sanity_checker::ScheduleSanityGC;
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(ScheduleSanityGC::<C::PlanType>::new(plan));
    }

    // Reference processing
    if !*plan.base().options.no_reference_types {
        use crate::util::reference_processor::{
            PhantomRefProcessing, SoftRefProcessing, WeakRefProcessing,
        };
        scheduler.work_buckets[WorkBucketStage::SoftRefClosure]
            .add(SoftRefProcessing::<C::ProcessEdgesWorkType>::new());
        scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
            .add(WeakRefProcessing::<C::ProcessEdgesWorkType>::new());
        scheduler.work_buckets[WorkBucketStage::PhantomRefClosure]
            .add(PhantomRefProcessing::<C::ProcessEdgesWorkType>::new());

        // VM-specific weak ref processing
        scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
            .add(VMProcessWeakRefs::<C::ProcessEdgesWorkType>::new());

        use crate::util::reference_processor::RefForwarding;
        if plan.constraints().needs_forward_after_liveness {
            scheduler.work_buckets[WorkBucketStage::RefForwarding]
                .add(RefForwarding::<C::ProcessEdgesWorkType>::new());
        }

        use crate::util::reference_processor::RefEnqueue;
        scheduler.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
    }

    // Finalization
    if !*plan.base().options.no_finalizer {
        use crate::util::finalizable_processor::{Finalization, ForwardFinalization};
        // finalization
        scheduler.work_buckets[WorkBucketStage::FinalRefClosure]
            .add(Finalization::<C::ProcessEdgesWorkType>::new());
        // forward refs
        if plan.constraints().needs_forward_after_liveness {
            scheduler.work_buckets[WorkBucketStage::FinalizableForwarding]
                .add(ForwardFinalization::<C::ProcessEdgesWorkType>::new());
        }
    }
}

impl<VM: VMBinding> Immix<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<Options>,
        scheduler: Arc<GCWorkScheduler<VM>>,
    ) -> Self {
        let mut heap = HeapMeta::new(&options);
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        let immix = Immix {
            immix_space: ImmixSpace::new(
                "immix",
                vm_map,
                mmapper,
                &mut heap,
                scheduler,
                global_metadata_specs.clone(),
            ),
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &IMMIX_CONSTRAINTS,
                global_metadata_specs,
            ),
            last_gc_was_defrag: AtomicBool::new(false),
        };

        {
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            immix
                .common
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            immix
                .immix_space
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        immix
    }
}
