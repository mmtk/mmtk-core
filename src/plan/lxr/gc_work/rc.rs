use super::super::LazySweepingJobsCounter;
use super::super::SurvivalRatioPredictorLocal;
use super::super::LXR;
use super::super::{LAZY_DECREMENTS, MATURE_EVACUATION, NO_EVAC, NURSERY_EVACUATION};
use super::tracing::LXRConcurrentTraceObjects;
use super::tracing::LXRStopTheWorldProcessEdges;
use super::ProcessEdgesBase;
use crate::plan::VectorQueue;
use crate::policy::immix::block::BlockState;
use crate::scheduler::gc_work::RootKind;
use crate::util::copy::CopySemantics;
use crate::util::linear_scan::UnstraddlableRegion;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::rc::*;
use crate::vm::slot::Slot;
use crate::{
    plan::concurrent::global::ConcurrentPlan,
    plan::concurrent::Pause,
    policy::{immix::block::Block, space::Space},
    scheduler::{GCWork, GCWorker, WorkBucketStage},
    util::{metadata::side_metadata, object_forwarding, ObjectReference},
    vm::*,
    MMTK,
};
use atomic::Ordering;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

pub struct ProcessIncs<VM: VMBinding, const KIND: EdgeKind> {
    /// Increments to process
    incs: Vec<VM::VMSlot>,
    /// Recursively generated new increments
    new_incs: VectorQueue<VM::VMSlot>,
    new_incs_count: u32,
    pause: Pause,
    in_cm: bool,
    no_evac: bool,
    pub root_kind: Option<RootKind>,
    depth: u32,
    lxr: &'static LXR<VM>,
    rc: RefCountHelper<VM>,
    survival_ratio_predictor_local: SurvivalRatioPredictorLocal,
    /// A slice of one promoted object's fields to count, rather than a buffer of increments:
    /// `(object, chunks, obj_in_defrag)`. See [`ProcessIncs::scan_nursery_object`].
    promoted_chunks: Option<(ObjectReference, std::ops::Range<usize>, bool)>,
}

unsafe impl<VM: VMBinding, const KIND: EdgeKind> Send for ProcessIncs<VM, KIND> {}

impl<VM: VMBinding, const KIND: EdgeKind> ProcessIncs<VM, KIND> {
    const CAPACITY: usize = 1024;
    /// How many of a large object's chunks one packet counts. Larger wastes the other workers'
    /// time, smaller pays the packet overhead more often; the point is only that it is a bound
    /// that does not grow with the object.
    const PROMOTED_CHUNK_SIZE: usize = 8192;
    const UNLOG_BITS: SideMetadataSpec = *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
        .as_spec()
        .extract_side_spec();

    fn __default(lxr: &'static LXR<VM>) -> Self {
        Self {
            incs: vec![],
            new_incs: VectorQueue::default(),
            new_incs_count: 0,
            lxr,
            pause: Pause::RefCount,
            in_cm: false,
            no_evac: false,
            depth: 1,
            rc: RefCountHelper::NEW,
            root_kind: None,
            survival_ratio_predictor_local: SurvivalRatioPredictorLocal::default(),
            promoted_chunks: None,
        }
    }

    fn add_new_slot(&mut self, worker: &mut GCWorker<VM>, s: VM::VMSlot) {
        self.new_incs.push(s);
        self.new_incs_count += 1;
        if self.new_incs_count as usize >= Self::CAPACITY {
            self.flush(worker);
        }
    }

    pub fn new(incs: Vec<VM::VMSlot>, lxr: &'static LXR<VM>) -> Self {
        Self {
            incs,
            ..Self::__default(lxr)
        }
    }

    /// Increment root objects that were reported as objects rather than as slots.
    ///
    /// This is the node-shaped counterpart of the slot path: incrementing is not enough
    /// on its own, because it is the zero-to-one transition that promotes an object,
    /// and promotion is what arms its field unlog bits and increments the objects it
    /// refers to. A bare increment leaves a nursery root's referents at zero, and they
    /// are then swept even though the root keeps them reachable.
    ///
    /// Returns `(counted, uncounted)`. `counted` is the objects whose count was raised,
    /// which is the set to record as the root set so the matching decrements are applied
    /// later. `uncounted` is the roots the plan never reference counts -- objects in the
    /// common plan's spaces, above all Julia's loaded sysimage -- which get no count and so
    /// must not be recorded for decrementing.
    ///
    /// Both sets still have to be traced. An uncounted root used to be dropped here
    /// outright, and then nothing ever scanned it: a tracing pause marks from the root sets
    /// this returns, and the sysimage object was in neither. Anything in the heap held only
    /// by a sysimage root was therefore never marked, and `SweepDeadCycles` -- which
    /// reclaims any counted object it finds unmarked -- zeroed its count while it was still
    /// reachable. Nothing in the bootstrap stage runs with a sysimage loaded, which is why
    /// this only ever showed up when building `Base`.
    pub fn process_root_nodes(
        &mut self,
        worker: &mut GCWorker<VM>,
        nodes: Vec<ObjectReference>,
    ) -> (Vec<ObjectReference>, Vec<ObjectReference>) {
        self.pause = self.lxr.current_pause().unwrap();
        self.in_cm = self.lxr.concurrent_work_in_progress();
        let mut roots = Vec::with_capacity(nodes.len());
        let mut uncounted = vec![];
        for o in nodes {
            if !self.lxr.is_rc_object(o) {
                uncounted.push(o);
                continue;
            }
            let los = self.lxr.los().in_space(o);
            if self.inc(o) {
                self.promote(worker, o, false, los, 0);
            }
            roots.push(o);
        }
        // Promotion above can queue further increments; hand them on rather than
        // dropping them.
        self.flush(worker);
        (roots, uncounted)
    }

    fn promote(
        &mut self,
        worker: &mut GCWorker<VM>,
        o: ObjectReference,
        copied: bool,
        los: bool,
        depth: u32,
    ) {
        let size = o.get_size::<VM>();

        if !los {
            let block = Block::containing(o);
            let in_nursery_block = block.get_state() == BlockState::Nursery;
            if !copied && in_nursery_block {
                block.set_as_in_place_promoted();
            }
            self.rc.promote_with_size(o, size);
            self.survival_ratio_predictor_local.record_promotion(size);
            if copied {
                self.survival_ratio_predictor_local
                    .record_copied_promotion(size);
            }
        } else {
            // println!("promote los {:?} {}", o, self.immix().is_marked(o));
        }
        // Don't mark copied objects in initial mark pause. The concurrent marker will do it (and can also resursively mark the old objects).
        if self.in_cm || self.pause == Pause::FinalMark {
            debug_assert!(self.lxr.is_marked(o), "{:?} is not marked", o);
        }
        self.scan_nursery_object(worker, o, los, !copied, depth, size);
    }

    fn record_mature_evac_remset2(
        &mut self,
        slot_in_defrag: bool,
        s: VM::VMSlot,
        o: ObjectReference,
    ) {
        if !(MATURE_EVACUATION && (self.in_cm || self.pause == Pause::FinalMark)) {
            return;
        }
        if !slot_in_defrag && self.lxr.in_defrag(o) {
            self.lxr.mature_evac_remset.record(s, o, self.lxr);
        }
    }

    fn record_mature_evac_remset(&mut self, s: VM::VMSlot, o: ObjectReference) {
        if !(MATURE_EVACUATION && (self.in_cm || self.pause == Pause::FinalMark)) {
            return;
        }
        self.record_mature_evac_remset2(self.lxr.address_in_defrag(s.to_address()), s, o);
    }

    fn scan_nursery_object(
        &mut self,
        worker: &mut GCWorker<VM>,
        o: ObjectReference,
        los: bool,
        in_place_promotion: bool,
        _depth: u32,
        size: usize,
    ) {
        let heap_bytes_per_unlog_byte = if VM::VMObjectModel::COMPRESSED_PTR_ENABLED {
            32usize
        } else {
            64
        };
        if los {
            let start =
                side_metadata::address_to_meta_address(&Self::UNLOG_BITS, o.to_raw_address())
                    .to_mut_ptr::<u8>();
            let limit = side_metadata::address_to_meta_address(
                &Self::UNLOG_BITS,
                (o.to_raw_address() + size).align_up(heap_bytes_per_unlog_byte),
            )
            .to_mut_ptr::<u8>();
            unsafe {
                let bytes = limit.offset_from(start) as usize;
                std::ptr::write_bytes(start, 0xffu8, bytes);
            }
            o.to_raw_address().unlog_field_relaxed::<VM>();
        } else if in_place_promotion {
            // Arm every unlog bit covering the object, starting from the granule that holds its
            // first byte.
            //
            // This used to start from `o + header_size` and then align *down*, meaning to skip
            // the header. But aligning down never moves forward, so whenever `o + header_size`
            // landed in the next granule -- i.e. whenever the reference address sits in the last
            // `header_size` bytes of one -- arming began one granule late and the bits covering
            // the object's first fields were never armed at all. A recycled line has those bits
            // cleared to `LOGGED`, so the barrier then treated those fields as already
            // snapshotted for the rest of the program: no decrement, no increment, and the
            // reference stored there was never counted. Objects keeping a reference in their
            // first words (`TypeMapEntry.next`, `Array.ref_`) were reclaimed while still live.
            //
            // Over-arming is harmless -- it only makes the barrier record a field it need not --
            // so cover the whole allocation and let the granule rounding fall where it may.
            let step = heap_bytes_per_unlog_byte << 2;
            let start = VM::VMObjectModel::ref_to_object_start(o);
            let aligned_end = (start + size).align_up(step);
            let mut cursor = start.align_down(step);
            let mut meta = side_metadata::address_to_meta_address(&Self::UNLOG_BITS, cursor);
            while cursor < aligned_end {
                unsafe { meta.store(0xffffffffu32) }
                meta += 4usize;
                cursor += step;
            }
        };
        // Promotion above arms the per-field unlog bits, which is what LXR's own barrier
        // consults. Bindings whose inlined write-barrier fast path cannot name the field
        // instead gate on the per-object log bit (Julia does this in `mmtk_gc_wb_fast`),
        // and nothing else sets it for an object promoted through the reference-counting
        // path rather than through tracing. Leaving it clear makes such a binding skip
        // the barrier for every mature object, losing the decrements and increments that
        // keep reachable objects alive.
        VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(o, Ordering::SeqCst);
        let obj_in_defrag = !los && Block::in_defrag_block(o);
        // One worker walks one object, and the walk is order-independent, so an object big
        // enough to dominate a pause is handed to the other workers in pieces instead. Julia's
        // 100M-element array of references took ~300ms to walk here, in a pause, with every
        // other worker idle: nothing spills into the increment buffers when the fields all
        // point at the same already-counted object, so buffer-level splitting cannot reach it.
        if let Some(count) = o.scan_chunk_count::<VM>() {
            if count > Self::PROMOTED_CHUNK_SIZE {
                let mut start = 0;
                while start < count {
                    let end = (start + Self::PROMOTED_CHUNK_SIZE).min(count);
                    let mut w = ProcessIncs::<VM, EDGE_KIND_NURSERY>::new(vec![], self.lxr);
                    w.depth = _depth + 1;
                    w.promoted_chunks = Some((o, start..end, obj_in_defrag));
                    worker.add_work(WorkBucketStage::Unconstrained, w);
                    start = end;
                }
                return;
            }
        }
        let tls = worker.tls.0;
        o.iterate_fields::<VM, _>(tls, |slot| {
            self.count_promoted_field(worker, o, obj_in_defrag, slot)
        });
    }

    /// Count one field of a just-promoted object `o`.
    ///
    /// An already-counted target is incremented in place. An uncounted one is deferred as a new
    /// increment, so that it is promoted in turn -- it is the zero-to-one transition that
    /// promotes, and promotion is what arms an object's field unlog bits.
    fn count_promoted_field(
        &mut self,
        worker: &mut GCWorker<VM>,
        o: ObjectReference,
        obj_in_defrag: bool,
        slot: VM::VMSlot,
    ) {
        let Some(target) = slot.load() else {
            return;
        };
        debug_assert!(
            target.to_raw_address().is_mapped(),
            "Unmapped obj {:?}.{:?} -> {:?}",
            o,
            slot,
            target
        );
        debug_assert!(
            target.is_in_any_space(),
            "Unmapped obj {:?}.{:?} -> {:?}",
            o,
            slot,
            target
        );
        let rc = self.rc.count(target);
        if rc == 0 {
            self.add_new_slot(worker, slot);
        } else {
            if rc != crate::util::rc::MAX_REF_COUNT {
                let _ = self.rc.inc(target);
            }
            self.record_mature_evac_remset2(obj_in_defrag, slot, target);
        }
    }

    #[cold]
    fn flush(&mut self, worker: &mut GCWorker<VM>) {
        if !self.new_incs.is_empty() {
            let new_incs = self.new_incs.take();
            let mut w = ProcessIncs::<VM, EDGE_KIND_NURSERY>::new(new_incs, self.lxr);
            w.depth += 1;
            worker.add_work(WorkBucketStage::Unconstrained, w);
        }
        self.new_incs_count = 0;
    }

    /// Return true if the object's ref count is incremented and the count was zero before the increment
    fn inc(&self, o: ObjectReference) -> bool {
        self.rc.inc(o) == Ok(0)
    }

    fn dont_evacuate(&self, o: ObjectReference, los: bool) -> bool {
        if los {
            return true;
        }
        // Skip mature object
        if self.rc.count(o) != 0 {
            return true;
        }
        // Skip recycled lines
        if Block::containing(o).get_state() != BlockState::Nursery {
            return true;
        }
        if cfg!(debug_assertions) {
            let cls = unsafe { (o.to_raw_address() + 8usize).load::<u32>() };
            assert!(cls != 0, "ERROR {:?} rc={}", o, self.rc.count(o));
        }
        false
    }

    fn process_inc_and_evacuate(
        &mut self,
        worker: &mut GCWorker<VM>,
        o: ObjectReference,
        depth: u32,
    ) -> ObjectReference {
        let los = self.lxr.los().in_space(o);
        if NURSERY_EVACUATION && !los && object_forwarding::is_forwarded_or_being_forwarded::<VM>(o)
        {
            while object_forwarding::is_being_forwarded::<VM>(o) {
                std::hint::spin_loop();
            }
            let new = if object_forwarding::is_forwarded::<VM>(o) {
                object_forwarding::read_forwarding_pointer::<VM>(o)
            } else {
                o
            };
            let promoted = self.inc(new);
            if promoted && new == o {
                self.promote(worker, o, false, los, depth);
            }
            return new;
        }
        if !NURSERY_EVACUATION || self.dont_evacuate(o, los) {
            if self.inc(o) {
                self.promote(worker, o, false, los, depth);
            }
            return o;
        }
        let forwarding_status = object_forwarding::attempt_to_forward::<VM>(o);
        if object_forwarding::state_is_forwarded_or_being_forwarded(forwarding_status) {
            // Object is moved to a new location.
            let new = object_forwarding::spin_and_get_forwarded_object::<VM>(o, forwarding_status);
            self.inc(new);
            new
        } else {
            let is_nursery = self.rc.count(o) == 0;
            if is_nursery && !self.no_evac {
                // Evacuate the object
                let new = object_forwarding::try_forward_object::<VM>(
                    o,
                    CopySemantics::DefaultCopy,
                    worker.get_copy_context_mut(),
                    |_new| {
                        #[cfg(feature = "vo_bit")]
                        {
                            // Set the VO bit of the new object.
                            crate::util::metadata::vo_bit::set_vo_bit(_new);
                            // Clear the VO bit of the old object.
                            // Note that sweeping can also clear the VO bit when the line is freed,
                            // but no RC inc/dec should be performed on the old object from now on.
                            // We clear it eagerly to detect inc/dec errors.
                            crate::util::metadata::vo_bit::unset_vo_bit(o);
                        }
                    },
                );
                if let Some(new) = new {
                    self.inc(new);
                    self.promote(worker, new, true, false, depth);
                    new
                } else {
                    warn!("to-space overflow");
                    // Object is not moved.
                    let promoted = self.inc(o);
                    object_forwarding::clear_forwarding_bits::<VM>(o);
                    if promoted {
                        self.promote(worker, o, false, los, depth);
                    }
                    NO_EVAC.store(true, Ordering::Relaxed);
                    self.no_evac = true;
                    o
                }
            } else {
                // Object is not moved.
                let promoted = self.inc(o);
                object_forwarding::clear_forwarding_bits::<VM>(o);
                if promoted {
                    self.promote(worker, o, false, los, depth);
                }
                o
            }
        }
    }

    /// Return `None` if the increment of the slot should be delayed
    fn unlog_and_load_rc_object<const K: EdgeKind>(
        &mut self,
        s: VM::VMSlot,
    ) -> Option<ObjectReference> {
        let o = s.load();
        // Re-arm the field so the next epoch's first write to it is recorded again. A slot with
        // no unlog bit has nothing to re-arm, and deriving a metadata address from one would
        // read unmapped memory, so the mapped-metadata test is the guard.
        //
        // It must be *that* test and not "is the slot inside the heap". Objects in the VM space
        // -- Julia's loaded sysimage, mapped well outside the heap range -- do have field unlog
        // bits, armed by `VMSpace::set_side_metadata`. Guarding on the heap range meant a
        // sysimage field was logged by its first barriered write and then never re-armed, so
        // every later write to it was invisible: references the sysimage stored into the heap
        // went uncounted and were reclaimed while live. That is the whole `sysbase` stage
        // failure -- ~2800 undercounted objects at the second collection, with referrers at
        // `0x740f…` addresses reporting `field_logged=Some(true)` forever.
        if K == EDGE_KIND_MATURE {
            let a = s.to_address();
            if side_metadata::address_to_meta_address(&Self::UNLOG_BITS, a).is_mapped() {
                a.unlog_field_relaxed::<VM>();
            }
        }
        o
    }

    fn process_slot<const K: EdgeKind>(
        &mut self,
        worker: &mut GCWorker<VM>,
        s: VM::VMSlot,
        depth: u32,
        add_root_to_remset: bool,
    ) -> Option<ObjectReference> {
        let o = match self.unlog_and_load_rc_object::<K>(s) {
            Some(o) => o,
            _ => {
                return None;
            }
        };
        // Objects the plan never reclaims carry no reference count, so there is
        // nothing to increment and nothing to evacuate. Reporting them as absent also
        // keeps them out of the recorded root set, which exists so that the matching
        // decrements can be applied later.
        if !self.lxr.is_rc_object(o) {
            return None;
        }
        let new = self.process_inc_and_evacuate(worker, o, depth);
        // Put this into remset if this is a mature slot, or a weak root
        if K != EDGE_KIND_ROOT || add_root_to_remset {
            self.record_mature_evac_remset(s, new);
        }
        if new != o {
            s.store(new)
        }
        Some(new)
    }

    fn process_incs<const K: EdgeKind>(
        &mut self,
        worker: &mut GCWorker<VM>,
        incs: AddressBuffer<'_, VM::VMSlot>,
        depth: u32,
        add_root_to_remset: bool,
    ) -> Option<Vec<ObjectReference>> {
        if K == EDGE_KIND_ROOT {
            // This used to reuse the increment buffer's allocation in place, writing the
            // resulting objects over the slots and handing the same pointer to
            // `Vec::from_raw_parts`. That is only sound when a slot and an
            // `ObjectReference` have identical layout. A VM whose slot type is larger --
            // Julia's `JuliaVMSlot` is an enum over a plain and an offset slot, three
            // words wide -- would hand the resulting `Vec` a capacity counted in slots,
            // and dropping it would free the allocation with a size three times too
            // small, corrupting the allocator. Collect into its own buffer instead.
            let mut roots = Vec::with_capacity(incs.len());
            for s in incs.iter() {
                if let Some(new) = self.process_slot::<K>(worker, *s, depth, add_root_to_remset) {
                    roots.push(new);
                }
            }
            if roots.is_empty() {
                None
            } else {
                Some(roots)
            }
        } else {
            for s in incs.iter() {
                self.process_slot::<K>(worker, *s, depth, false);
            }
            None
        }
    }
}

pub type EdgeKind = u8;
pub const EDGE_KIND_ROOT: u8 = 0;
pub const EDGE_KIND_NURSERY: u8 = 1;
pub const EDGE_KIND_MATURE: u8 = 2;

enum AddressBuffer<'a, S: Slot> {
    Owned(Vec<S>),
    Ref(&'a mut Vec<S>),
}

impl<S: Slot> Deref for AddressBuffer<'_, S> {
    type Target = Vec<S>;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(x) => x,
            Self::Ref(x) => x,
        }
    }
}

impl<S: Slot> DerefMut for AddressBuffer<'_, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Owned(x) => x,
            Self::Ref(x) => x,
        }
    }
}

impl<VM: VMBinding, const KIND: EdgeKind> GCWork<VM> for ProcessIncs<VM, KIND> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        self.lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        self.pause = self.lxr.current_pause().unwrap();
        self.in_cm = self.lxr.concurrent_work_in_progress();
        if NO_EVAC.load(Ordering::Relaxed) {
            self.no_evac = true;
        } else {
            let over_space = mmtk.get_plan().get_used_pages()
                - mmtk.get_plan().get_collection_reserved_pages()
                > mmtk.get_plan().get_total_pages();
            if over_space {
                self.no_evac = true;
                NO_EVAC.store(true, Ordering::Relaxed);
            }
        }
        // A slice of a large promoted object's fields, rather than an increment buffer. The
        // parent's `promote` has already armed the unlog bits and marked the object; all that is
        // left is to count the fields, and the increments this generates are flushed by the same
        // path as any other.
        if let Some((o, chunks, obj_in_defrag)) = self.promoted_chunks.take() {
            o.iterate_fields_in_chunks::<VM, _>(chunks, |slot| {
                self.count_promoted_field(worker, o, obj_in_defrag, slot)
            });
        }
        // Process main buffer
        let root_slots = if KIND == EDGE_KIND_ROOT
            && (self.pause == Pause::FinalMark || self.pause == Pause::Full)
        {
            self.incs.clone()
        } else {
            vec![]
        };
        let roots = {
            let incs = std::mem::take(&mut self.incs);
            self.process_incs::<KIND>(worker, AddressBuffer::Owned(incs), self.depth, false)
        };
        // Seed the stop-the-world trace from the root *slots*, independently of whether any
        // of them turned out to hold a reference counted object. `process_incs` reports only
        // the counted objects, and a packet holding nothing but sysimage roots reports none
        // at all -- so gating the trace on that result silently dropped those roots from
        // marking, and anything held only by them was swept while reachable.
        if (self.pause == Pause::FinalMark || self.pause == Pause::Full)
            && !root_slots.is_empty()
            && self.root_kind != Some(RootKind::Weak)
        {
            if self.pause == Pause::FinalMark {
                let mut w = LXRStopTheWorldProcessEdges::<_, false>::new(
                    root_slots,
                    true,
                    mmtk,
                    WorkBucketStage::Closure,
                );
                w.root_kind = self.root_kind;
                worker.add_work(WorkBucketStage::Closure, w)
            } else {
                let mut w = LXRStopTheWorldProcessEdges::<_, true>::new(
                    root_slots,
                    true,
                    mmtk,
                    WorkBucketStage::Closure,
                );
                w.root_kind = self.root_kind;
                worker.add_work(WorkBucketStage::Closure, w)
            };
        }
        if let Some(roots) = roots {
            if self.lxr.cm_enabled()
                && self.pause == Pause::InitialMark
                && !self.root_kind.unwrap().should_skip_mark_and_decs()
            {
                if cfg!(any(feature = "sanity", debug_assertions)) {
                    for r in &roots {
                        assert!(
                            r.to_raw_address().is_mapped(),
                            "Invalid object {:?}: address is not mapped",
                            r
                        );
                    }
                }
                worker.scheduler().work_buckets[WorkBucketStage::ConcurrentResumable]
                    .add(LXRConcurrentTraceObjects::new(roots.clone(), mmtk));
            }
            if self.pause != Pause::FinalMark
                && self.pause != Pause::Full
                && !self.root_kind.unwrap().should_skip_decs()
            {
                self.lxr.curr_roots.read().unwrap().push(roots);
            }
        }
        // Process recursively generated buffer
        let mut depth = self.depth;
        let mut incs = vec![];
        // Hand half of each generation to another worker.
        //
        // Without this the promotion trace is a chain, not a tree. `add_new_slot` spills a packet
        // every `CAPACITY` (1024) slots, and a packet consuming 1024 slots promotes ~512 objects
        // whose fields are ~1024 new slots -- so each packet produces almost exactly one successor.
        // Measured on `tree_mutable`: 2.08M increments in 1984 packets, i.e. 1048 slots each, formed
        // as 13 chains (one per root packet) about 152 packets long. Parallelism was therefore
        // capped at 13 no matter how many workers existed, and measured ~3x because the root buffers
        // are uneven -- `ProcessIncs` CPU over pause wall stayed at ~3x whether 4, 16 or 64 workers
        // were available.
        //
        // Splitting each generation converts the chain into a binary tree, whose depth is
        // logarithmic in the generation size rather than linear in it.
        let split = crate::plan::lxr::active_packet_split();
        while !self.new_incs.is_empty() {
            self.new_incs_count = 0;
            depth += 1;
            incs.clear();
            self.new_incs.swap(&mut incs);
            if incs.len() > split {
                let (a, b) = incs.split_at(incs.len() / 2);
                let mut w = ProcessIncs::<VM, EDGE_KIND_NURSERY>::new(b.to_vec(), self.lxr);
                w.depth = depth;
                worker.add_work(WorkBucketStage::Unconstrained, w);
                incs = a.to_vec();
            }
            if !incs.is_empty() {
                self.process_incs::<EDGE_KIND_NURSERY>(
                    worker,
                    AddressBuffer::Ref(&mut incs),
                    depth,
                    false,
                );
            }
        }
        self.survival_ratio_predictor_local.sync();
    }
}

pub struct ProcessDecs<VM: VMBinding> {
    /// Decrements to process
    decs: Option<Vec<ObjectReference>>,
    decs_arc: Option<Arc<Vec<ObjectReference>>>,
    /// Recursively generated new decrements
    new_decs: VectorQueue<ObjectReference>,
    counter: LazySweepingJobsCounter,
    mark_objects: VectorQueue<ObjectReference>,
    mark_dead_objects: bool,
    mature_sweeping_in_progress: bool,
    rc: RefCountHelper<VM>,
}

impl<VM: VMBinding> ProcessDecs<VM> {
    pub fn new(decs: Vec<ObjectReference>, counter: LazySweepingJobsCounter) -> Self {
        Self {
            decs: Some(decs),
            decs_arc: None,
            new_decs: VectorQueue::default(),
            counter,
            mark_objects: VectorQueue::default(),
            mark_dead_objects: false,
            mature_sweeping_in_progress: false,
            rc: RefCountHelper::NEW,
        }
    }

    pub fn new_arc(decs: Arc<Vec<ObjectReference>>, counter: LazySweepingJobsCounter) -> Self {
        Self {
            decs: None,
            decs_arc: Some(decs),
            new_decs: VectorQueue::default(),
            counter,
            mark_objects: VectorQueue::default(),
            mark_dead_objects: false,
            mature_sweeping_in_progress: false,
            rc: RefCountHelper::NEW,
        }
    }

    fn recursive_dec(&mut self, worker: &mut GCWorker<VM>, o: ObjectReference) {
        self.new_decs.push(o);
        if self.new_decs.is_full() {
            self.flush(worker)
        }
    }

    fn new_work(&self, worker: &mut GCWorker<VM>, w: ProcessDecs<VM>) {
        worker.add_work(WorkBucketStage::Unconstrained, w);
    }

    fn flush(&mut self, worker: &mut GCWorker<VM>) {
        let mmtk = worker.mmtk;
        if !self.new_decs.is_empty() {
            let new_decs = self.new_decs.take();
            let w = ProcessDecs::new(new_decs, self.counter.clone_with_decs());
            self.new_work(worker, w);
        }
        if !self.mark_objects.is_empty() {
            let objects = self.mark_objects.take();
            let w = LXRConcurrentTraceObjects::new(objects, mmtk);
            if LAZY_DECREMENTS {
                worker.add_work(WorkBucketStage::Unconstrained, w);
            } else {
                worker.scheduler().work_buckets[WorkBucketStage::ConcurrentResumable].add(w);
            }
        }
    }

    #[cold]
    fn process_dead_object(
        &mut self,
        worker: &mut GCWorker<VM>,
        o: ObjectReference,
        lxr: &LXR<VM>,
    ) -> bool {
        if self.mark_dead_objects {
            lxr.mark(o);
        }
        // Recursively decrease field ref counts
        let tls = worker.tls.0;
        o.iterate_fields::<VM, _>(tls, |slot| {
            if let Some(x) = slot.load() {
                // println!(" -- rec dec {:?}.{:?} -> {:?}", o, slot, x);
                let rc = self.rc.count(x);
                if rc != MAX_REF_COUNT && rc != 0 {
                    self.recursive_dec(worker, x);
                }
                if self.mark_dead_objects && !lxr.is_marked(x) {
                    if cfg!(any(feature = "sanity", debug_assertions)) {
                        assert!(
                            x.to_raw_address().is_mapped(),
                            "Invalid object {:?}.{:?} -> {:?}: address is not mapped",
                            o,
                            slot,
                            x
                        );
                    }
                    self.mark_objects.push(x);
                    if self.mark_objects.is_full() {
                        self.flush(worker);
                    }
                }
            }
        });
        let in_ix_space = lxr.immix_space.in_space(o);
        debug_assert!(
            in_ix_space || lxr.common.los.in_space(o),
            "{:?} is not reference counted, so its count can never reach zero",
            o
        );
        if in_ix_space {
            // Clear the VO bit if `o` is in the immix space.
            // Note that if the object is in the LOS,
            // the VO bit will be cleared in `LargeObjectSpace::release_object`.
            #[cfg(feature = "vo_bit")]
            crate::util::metadata::vo_bit::unset_vo_bit(o);

            self.rc.unmark_straddle_object(o);
        }
        if RefCountHelper::<VM>::SANITY {
            unsafe { o.to_raw_address().store(0xdeadusize) };
        }
        if in_ix_space {
            let block = Block::containing(o);
            lxr.add_to_possibly_dead_mature_blocks(block, false);
            false
        } else {
            // Only the large object space frees objects individually, and the caller
            // uses this to decide whether to ask it to.
            lxr.common.los.in_space(o)
        }
    }

    fn process_decs(&mut self, worker: &mut GCWorker<VM>, decs: &[ObjectReference], lxr: &LXR<VM>) {
        for o in decs.iter() {
            if self.rc.is_dead_or_stuck(*o)
                || (self.mature_sweeping_in_progress && !lxr.is_marked(*o))
            {
                continue;
            }
            let o = if MATURE_EVACUATION && object_forwarding::is_forwarded::<VM>(*o) {
                object_forwarding::read_forwarding_pointer::<VM>(*o)
            } else {
                *o
            };
            let mut dead = false;
            let mut is_los = false;
            let mut already_run = false;
            let result = self.rc.clone().fetch_update(o, |c| {
                if already_run {
                    log::warn!("fetch_update is re-run! o: {o}");
                } else {
                    already_run = true;
                }
                if c == 1 && !dead {
                    dead = true;
                    is_los = self.process_dead_object(worker, o, lxr);
                }
                debug_assert!(c <= MAX_REF_COUNT);
                if c == 0 || c == MAX_REF_COUNT {
                    None /* sticky */
                } else {
                    Some(c - 1)
                }
            });
            if result == Ok(1) && is_los {
                lxr.los().rc_free(o);
            }
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for ProcessDecs<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        self.mark_dead_objects = if LAZY_DECREMENTS {
            lxr.concurrent_work_in_progress() && lxr.previous_pause() != Some(Pause::InitialMark)
        } else {
            lxr.concurrent_work_in_progress() && lxr.current_pause() != Some(Pause::InitialMark)
        };
        self.mature_sweeping_in_progress = if LAZY_DECREMENTS {
            lxr.previous_pause() == Some(Pause::FinalMark)
                || lxr.current_pause() == Some(Pause::Full)
        } else {
            lxr.current_pause() == Some(Pause::FinalMark)
                || lxr.current_pause() == Some(Pause::Full)
        };
        if let Some(decs) = std::mem::take(&mut self.decs) {
            self.process_decs(worker, &decs, lxr);
        } else if let Some(decs) = std::mem::take(&mut self.decs_arc) {
            self.process_decs(worker, &decs, lxr);
        }
        let mut decs = vec![];
        while !self.new_decs.is_empty() {
            decs.clear();
            self.new_decs.swap(&mut decs);
            self.process_decs(worker, &decs, lxr);
        }
        self.flush(worker);
    }
}

pub struct CollectRoots<VM: VMBinding> {
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding> CollectRoots<VM> {
    pub fn new(
        slots: Vec<VM::VMSlot>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        debug_assert!(roots);
        let base = ProcessEdgesBase::new(slots, roots, mmtk, bucket);
        Self { base }
    }
}

impl<VM: VMBinding> GCWork<VM> for CollectRoots<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.set_worker(worker);
        if !self.slots.is_empty() {
            let lxr = self.mmtk().get_plan().downcast_ref::<LXR<VM>>().unwrap();
            let roots = std::mem::take(&mut self.slots);
            let mut w = ProcessIncs::<_, EDGE_KIND_ROOT>::new(roots, lxr);
            w.root_kind = self.root_kind;
            GCWork::do_work(&mut w, self.worker(), self.mmtk());
        }
    }
}

impl<VM: VMBinding> Deref for CollectRoots<VM> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for CollectRoots<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
