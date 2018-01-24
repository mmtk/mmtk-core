use ::util::Address;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::entrypoint::*;
use super::JTOC_BASE;
use ::plan::{TraceLocal, Plan, SelectedPlan, ParallelCollector};

use super::scheduling::VMScheduling;

const DEBUG: bool = false;
const FILTER: bool = true;

const LOG_CHUNK_BYTES: usize = 12;
const CHUNK_BYTES: usize = 1 << LOG_CHUNK_BYTES;
const LONGENCODING_MASK: usize = 0x1;
const RUN_MASK: usize = 0x2;
const MAX_RUN: usize = (1 << 8) - 1;
const LONGENCODING_OFFSET_BYTES: usize = 4;
const GUARD_REGION: usize = LONGENCODING_OFFSET_BYTES + 1; /* long offset + run encoding */

static ROOTS: AtomicUsize = AtomicUsize::new(0);
static REFS: AtomicUsize = AtomicUsize::new(0);

pub fn scan_boot_image<T: TraceLocal>(trace: &mut T, thread_id: usize) {
    unsafe {
        let boot_record = Address::from_usize((JTOC_BASE + THE_BOOT_RECORD_FIELD_OFFSET)
            .load::<usize>());
        let map_start = Address::from_usize((boot_record + BOOT_IMAGE_R_MAP_START_OFFSET)
            .load::<usize>());
        let map_end = Address::from_usize((boot_record + BOOT_IMAGE_R_MAP_END_OFFSET)
            .load::<usize>());
        let image_start = Address::from_usize((boot_record + BOOT_IMAGE_DATA_START_FIELD_OFFSET)
            .load::<usize>());

        let thread = VMScheduling::thread_from_id(thread_id);
        let system_thread = Address::from_usize(
            (thread + SYSTEM_THREAD_FIELD_OFFSET).load::<usize>());
        let collector = &*((system_thread + WORKER_INSTANCE_FIELD_OFFSET)
            .load::<*const <SelectedPlan as Plan>::CollectorT>());

        let stride = collector.parallel_worker_count() << LOG_CHUNK_BYTES;
        let start = collector.parallel_worker_ordinal() << LOG_CHUNK_BYTES;
        let mut cursor = map_start + start;

        ROOTS.store(0, Ordering::Relaxed);
        ROOTS.store(0, Ordering::Relaxed);

        while cursor < map_end {
            process_chunk(cursor, image_start, map_start, map_end, trace);
            cursor += stride;
        }
    }
}

fn process_chunk<T: TraceLocal>(chunk_start: Address, image_start: Address,
                                map_start: Address, map_end: Address, trace: &mut T) {
    unimplemented!();
}

