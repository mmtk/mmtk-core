use crate::plan::generational::global::GenerationalPlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::immix;
use crate::plan::GcStatus;
use crate::plan::PlanConstraints;
use crate::policy::sft::SFT;
use crate::policy::space::Space;
use crate::util::copy::CopyConfig;
use crate::util::copy::CopySelector;
use crate::util::copy::CopySemantics;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use crate::Plan;

use atomic::Ordering;
use std::sync::atomic::AtomicBool;

use mmtk_macros::PlanTraceObject;

use super::gc_work::StickyImmixMatureGCWorkContext;
use super::gc_work::StickyImmixNurseryGCWorkContext;

#[derive(PlanTraceObject)]
pub struct StickyImmix<VM: VMBinding> {
    #[fallback_trace]
    pub(in crate::plan::sticky::immix) immix: immix::Immix<VM>,
    gc_full_heap: AtomicBool,
    next_gc_full_heap: AtomicBool,
}

pub const STICKY_IMMIX_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    needs_log_bit: true,
    barrier: crate::plan::BarrierSelector::ObjectBarrier,
    ..immix::IMMIX_CONSTRAINTS
};

impl<VM: VMBinding> crate::plan::generational::global::SupportNurseryGC<VM> for StickyImmix<VM> {
    fn is_object_in_nursery(&self, object: crate::util::ObjectReference) -> bool {
        self.immix.immix_space.in_space(object)
            && !self
                .immix
                .immix_space
                .is_marked_with_current_mark_state(object)
    }

    // This check is used for memory slice copying barrier, where we only know addresses instead of objects.
    // As sticky immix needs object metadata to know if an object is an nursery object or not, we cannot really tell
    // whether an address is in nursery or not. In this case, we just return false -- this is a conservative return value
    // for the memory slice copying barrier. It means we will treat the object as if it is in mature space, and will
    // push it to the remembered set.
    fn is_address_in_nursery(&self, _addr: crate::util::Address) -> bool {
        false
    }

    fn trace_object_nursery<Q: crate::ObjectQueue>(
        &self,
        queue: &mut Q,
        object: crate::util::ObjectReference,
        worker: &mut crate::scheduler::GCWorker<VM>,
    ) -> crate::util::ObjectReference {
        if self.immix.immix_space.in_space(object) {
            if !self.is_object_in_nursery(object) {
                // Mature object
                trace!("Immix mature object {}, skip", object);
                return object;
            } else {
                let object = if crate::policy::immix::PREFER_COPY_ON_NURSERY_GC {
                    let ret = self.immix.immix_space.trace_object_with_opportunistic_copy(
                        queue,
                        object,
                        CopySemantics::DefaultCopy,
                        worker,
                        true,
                    );
                    trace!(
                        "Immix nursery object {} is being traced with opportunistic copy {}",
                        object,
                        if ret == object {
                            "".to_string()
                        } else {
                            format!(" -> new object {}", ret)
                        }
                    );
                    ret
                } else {
                    trace!(
                        "Immix nursery object {} is being traced without moving",
                        object
                    );
                    self.immix
                        .immix_space
                        .trace_object_without_moving(queue, object)
                };

                return object;
            }
        }

        if self.immix.common().get_los().in_space(object) {
            return self
                .immix
                .common()
                .get_los()
                .trace_object::<Q>(queue, object);
        }

        warn!(
            "Object {} is not in nursery or in LOS, it is not traced!",
            object
        );
        object
    }
}

impl<VM: VMBinding> Plan for StickyImmix<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static crate::plan::PlanConstraints {
        &STICKY_IMMIX_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::Immix(0),
                // CopySemantics::PromoteToMature => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::Immix(0), &self.immix.immix_space)],
            constraints: &STICKY_IMMIX_CONSTRAINTS,
        }
    }

    fn base(&self) -> &crate::plan::global::BasePlan<Self::VM> {
        self.immix.base()
    }

    fn generational(
        &self,
    ) -> Option<&dyn crate::plan::generational::global::GenerationalPlan<VM = Self::VM>> {
        Some(self)
    }

    fn common(&self) -> &CommonPlan<Self::VM> {
        self.immix.common()
    }

    fn force_full_heap_collection(&self) {
        self.next_gc_full_heap.store(true, Ordering::SeqCst);
    }

    fn last_collection_full_heap(&self) -> bool {
        self.gc_full_heap.load(Ordering::SeqCst)
    }

    fn schedule_collection(&'static self, scheduler: &crate::scheduler::GCWorkScheduler<Self::VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);

        let is_full_heap = self.requires_full_heap_collection();
        self.gc_full_heap.store(is_full_heap, Ordering::SeqCst);

        if !is_full_heap {
            info!("Nursery GC");
            // nursery GC -- we schedule it
            scheduler.schedule_common_work::<StickyImmixNurseryGCWorkContext<VM>>(self);
        } else {
            use crate::plan::immix::Immix;
            use crate::policy::immix::{TRACE_KIND_DEFRAG, TRACE_KIND_FAST};
            // self.immix.schedule_collection(scheduler);
            Immix::schedule_immix_collection::<
                StickyImmixMatureGCWorkContext<VM, TRACE_KIND_FAST>,
                StickyImmixMatureGCWorkContext<VM, TRACE_KIND_DEFRAG>,
            >(self, self, &self.immix.immix_space, scheduler);
        }
    }

    fn get_spaces(&self) -> Vec<&dyn crate::policy::space::Space<Self::VM>> {
        self.immix.get_spaces()
    }

    fn get_allocator_mapping(
        &self,
    ) -> &'static enum_map::EnumMap<crate::AllocationSemantics, crate::util::alloc::AllocatorSelector>
    {
        &super::mutator::ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: crate::util::VMWorkerThread) {
        if self.is_current_gc_nursery() {
            info!("Prepare nursery");
            // Prepare both large object space and immix space
            self.immix.immix_space.prepare(false);
            self.immix.common.los.prepare(false);
        } else {
            info!("Prepare full heap");
            self.immix.prepare(tls);
        }
    }

    fn release(&mut self, tls: crate::util::VMWorkerThread) {
        if self.is_current_gc_nursery() {
            info!("Release nursery");
            let was_defrag = self.immix.immix_space.release(false);
            self.immix
                .last_gc_was_defrag
                .store(was_defrag, Ordering::Relaxed);
            self.immix.common.los.release(false);
        } else {
            info!("Release full heap");
            self.immix.release(tls);
        }
    }

    fn end_of_gc(&mut self, _tls: crate::util::opaque_pointer::VMWorkerThread) {
        let next_gc_full_heap =
            crate::plan::generational::global::CommonGenPlan::should_next_gc_be_full_heap(self);
        self.next_gc_full_heap
            .store(next_gc_full_heap, Ordering::Relaxed);
    }

    fn collection_required(
        &self,
        space_full: bool,
        space: Option<&dyn crate::policy::space::Space<Self::VM>>,
    ) -> bool {
        let nursery_full =
            self.immix.immix_space.get_pages_allocated() > self.options().get_max_nursery_pages();
        if space_full && space.is_some() && space.unwrap().name() == self.immix.immix_space.name() {
            self.next_gc_full_heap.store(true, Ordering::SeqCst);
        }
        self.immix.collection_required(space_full, space) || nursery_full
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.immix.get_collection_reserved_pages() + self.immix.immix_space.defrag_headroom_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.immix.get_used_pages()
    }

    fn sanity_check_object(&self, object: crate::util::ObjectReference) {
        if self.is_current_gc_nursery() {
            if self.immix.immix_space.in_space(object) {
                // Every object should be logged
                if !VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                    .is_unlogged::<VM>(object, Ordering::SeqCst)
                {
                    self.get_spaces().iter().for_each(|s| {
                        crate::policy::space::print_vm_map(*s, &mut std::io::stdout()).unwrap();
                    });
                    panic!("Object {} is not unlogged (all objects that have been traced should be unlogged/mature)", object);
                }
                if !self
                    .immix
                    .immix_space
                    .is_marked_with_current_mark_state(object)
                {
                    self.get_spaces().iter().for_each(|s| {
                        crate::policy::space::print_vm_map(*s, &mut std::io::stdout()).unwrap();
                    });
                    panic!("Object {} is not marked (all objects that have been traced should be marked)", object);
                }
            } else if self.immix.common.los.in_space(object) {
                // Every object should be logged
                if !VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                    .is_unlogged::<VM>(object, Ordering::SeqCst)
                {
                    self.get_spaces().iter().for_each(|s| {
                        crate::policy::space::print_vm_map(*s, &mut std::io::stdout()).unwrap();
                    });
                    panic!("LOS Object {} is not unlogged (all objects that have been traced should be unlogged/mature)", object);
                }
                if !self.immix.common.los.is_live(object) {
                    self.get_spaces().iter().for_each(|s| {
                        crate::policy::space::print_vm_map(*s, &mut std::io::stdout()).unwrap();
                    });
                    panic!("LOS Object {} is not marked", object);
                }
            }
        }
    }
}

impl<VM: VMBinding> GenerationalPlan for StickyImmix<VM> {
    fn is_current_gc_nursery(&self) -> bool {
        !self.gc_full_heap.load(Ordering::SeqCst)
    }

    fn get_mature_physical_pages_available(&self) -> usize {
        self.immix.immix_space.available_physical_pages()
    }

    fn get_mature_reserved_pages(&self) -> usize {
        self.immix.immix_space.reserved_pages()
    }
}

impl<VM: VMBinding> StickyImmix<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &STICKY_IMMIX_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(
                &crate::plan::generational::new_generational_global_metadata_specs::<VM>(),
            ),
        };
        Self {
            immix: immix::Immix::new_with_plan_args(plan_args),
            gc_full_heap: AtomicBool::new(false),
            next_gc_full_heap: AtomicBool::new(false),
        }
    }

    fn requires_full_heap_collection(&self) -> bool {
        // Separate each condition so the code is clear
        #[allow(clippy::if_same_then_else, clippy::needless_bool)]
        if self
            .immix
            .common
            .base
            .user_triggered_collection
            .load(Ordering::SeqCst)
            && *self.immix.common.base.options.full_heap_system_gc
        {
            // User triggered collection, and we force full heap for user triggered collection
            true
        } else if self.next_gc_full_heap.load(Ordering::SeqCst)
            || self
                .immix
                .common
                .base
                .cur_collection_attempts
                .load(Ordering::SeqCst)
                > 1
        {
            // Forces full heap collection
            true
        } else {
            false
        }
    }
}
