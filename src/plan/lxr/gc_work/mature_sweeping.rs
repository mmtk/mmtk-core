use std::ops::Range;
use std::sync::atomic::Ordering;

use crate::plan::lxr::{LazySweepingJobsCounter, LXR};
use crate::policy::immix::block::{Block, BlockState};
use crate::policy::immix::line::Line;
use crate::policy::immix::ImmixSpace;
use crate::scheduler::{GCWork, GCWorker};
use crate::util::heap::chunk_map::Chunk;
use crate::util::linear_scan::Region;
use crate::util::rc::{self, RefCountHelper};
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::MMTK;

/// Chunk sweeping work packet.
pub struct SweepDeadCycles<VM: VMBinding> {
    chunks: Range<Chunk>,
    _counter: LazySweepingJobsCounter,
    rc: RefCountHelper<VM>,
}

#[allow(unused)]
impl<VM: VMBinding> SweepDeadCycles<VM> {
    const CAPACITY: usize = 1024;

    pub fn new(chunks: Range<Chunk>, counter: LazySweepingJobsCounter) -> Self {
        Self {
            chunks,
            _counter: counter,
            rc: RefCountHelper::NEW,
        }
    }

    fn process_dead_object(&mut self, o: ObjectReference) {
        if RefCountHelper::<VM>::SANITY {
            unsafe {
                o.to_raw_address().store(0xdeadusize);
            }
        }
        if crate::plan::lxr::check_incs() && !crate::plan::lxr::object_is_plausible(o) {
            self.report_bogus_dead_object(o);
        }

        // Clear the VO bit.
        // Note that if the object is in the LOS,
        // the VO bit will be cleared in `LargeObjectSpace::release_object`.
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::unset_vo_bit(o);

        self.rc.unmark_straddle_object(o);
        self.rc.set(o, 0);
    }

    /// Report a granule the linear scan took for a dead object although it does not hold one.
    ///
    /// The scan can only distinguish an object start from a synthetic line-occupancy mark by the
    /// straddle bit, so anything that carries a count without either being an object or being
    /// marked straddled lands here. What is needed is the *shape* of the stray count: where it
    /// sits within its line, what its neighbours hold, and what the straddle bits say.
    #[cold]
    fn report_bogus_dead_object(&self, o: ObjectReference) {
        let a = o.to_raw_address();
        let line = Line::from_unaligned_address(a);
        eprintln!("[lxr] sweep: granule at {a} is not an object");
        eprintln!(
            "  count={} line_aligned={} offset_in_line={} straddle(granule)={} straddle(line)={}",
            self.rc.count(o),
            Line::is_aligned(a),
            a - line.start(),
            self.rc.object_is_in_straddle_line_no_rc_check(o),
            self.rc
                .object_is_in_straddle_line_no_rc_check(unsafe {
                    ObjectReference::from_raw_address_unchecked(line.start())
                }),
        );
        eprint!("  neighbour counts =");
        for i in -4i64..=4 {
            let n = a.as_usize() as i64 + i * (rc::MIN_OBJECT_SIZE as i64);
            let n = unsafe { crate::util::Address::from_usize(n as usize) };
            let c = self.rc.count_by_address(n);
            eprint!("{}{}", if i == 0 { " >" } else { " " }, c);
        }
        eprintln!();
        eprint!("  words     =");
        for i in -4i64..=4 {
            let w = a.as_usize() as i64 + i * 8;
            let w = unsafe { crate::util::Address::from_usize(w as usize) };
            eprint!("{}{:#x}", if i == 0 { " >" } else { " " }, unsafe {
                w.load::<usize>()
            });
        }
        eprintln!();
    }

    fn process_block(&mut self, block: Block, lxr: &LXR<VM>, immix_space: &ImmixSpace<VM>) {
        let mut has_dead_object = false;
        let mut has_live = false;
        let mut cursor = block.start();
        let limit = block.end();
        while cursor < limit {
            let cur_cursor = cursor;
            cursor += rc::MIN_OBJECT_SIZE;
            let c = self.rc.count_by_address(cur_cursor);
            if c != 0 {
                // Safety: cur_cursor is either a valid object reference, or a straddle line
                let o = unsafe { ObjectReference::from_raw_address_unchecked(cur_cursor) };
                if !immix_space.is_marked(o) {
                    // A straddle bit on this granule means the count is a synthetic line-occupancy
                    // mark, not an object: `set_occupied_line_marks` writes one for every line an
                    // object touches besides the one holding its own count, so the hole finder does
                    // not read those lines as free. Sizing one reads a type tag from whatever
                    // precedes it.
                    //
                    // The bit is checked at the granule, not at the line. The mark for the line
                    // holding an object's header sits at the header word; for Julia, whose reference
                    // address is one word past the allocation start, that lands on the last word of
                    // the preceding line whenever an object begins exactly on a line boundary. Those
                    // granules (`offset_in_line=248`) are what crashed the sweep, and a per-line bit
                    // could not describe them.
                    if self.rc.object_is_in_straddle_line_no_rc_check(o) {
                        continue;
                    }
                    std::sync::atomic::fence(Ordering::SeqCst);
                    if self.rc.count(o) == 0 {
                        continue;
                    }
                    crate::plan::lxr::SWEEP_ZEROED.fetch_add(1, Ordering::Relaxed);
                    self.process_dead_object(o);
                    has_dead_object = true;
                } else {
                    crate::plan::lxr::SWEEP_KEPT_MARKED.fetch_add(1, Ordering::Relaxed);
                    has_live = true;
                }
            }
        }
        if has_dead_object || !has_live {
            lxr.add_to_possibly_dead_mature_blocks(block, false);
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for SweepDeadCycles<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        let immix_space = &lxr.immix_space;
        let num_chunks = (self.chunks.end.start() - self.chunks.start.start()) >> Chunk::LOG_BYTES;
        let ix_space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        for i in 0..num_chunks {
            let chunk = self.chunks.start.next_nth(i);
            if !ix_space.chunk_map.is_allocated(chunk) {
                continue;
            }

            for block in chunk
                .iter_region::<Block>()
                .filter(|block| block.get_state() != BlockState::Unallocated)
            {
                if block.is_defrag_source() || block.get_state() == BlockState::Nursery {
                    continue;
                } else {
                    self.process_block(block, lxr, immix_space)
                }
            }
        }
    }
}

pub struct RCSweepMatureAfterSATBLOS {
    _counter: LazySweepingJobsCounter,
}

impl RCSweepMatureAfterSATBLOS {
    pub fn new(counter: LazySweepingJobsCounter) -> Self {
        Self { _counter: counter }
    }
}

impl<VM: VMBinding> GCWork<VM> for RCSweepMatureAfterSATBLOS {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let los = mmtk.get_plan().common().get_los();
        los.sweep_rc_mature_objects_after_satb(&|o| los.is_marked(o) || los.rc.count(o) == 0);
    }
}
