use std::sync::atomic::{AtomicUsize, Ordering};

use libc::c_void;
use mmtk::util::Address;
use mmtk::util::OpaquePointer;
use mmtk::{TraceLocal, Plan, SelectedPlan, ParallelCollector};
use mmtk::vm::unboxed_size_constants::*;
use mmtk::vm::ActivePlan;
use mmtk::util::conversions;

use collection::VMCollection;
use active_plan::VMActivePlan;
use java_size_constants::*;
use entrypoint::*;
use JTOC_BASE;

const DEBUG: bool = false;
const FILTER: bool = true;

const LOG_CHUNK_BYTES: usize = 12;
const CHUNK_BYTES: usize = 1 << LOG_CHUNK_BYTES;
const LONGENCODING_MASK: usize = 0x1;
const RUN_MASK: usize = 0x2;
const MAX_RUN: usize = (1 << BITS_IN_BYTE) - 1;
const LONGENCODING_OFFSET_BYTES: usize = 4;
const GUARD_REGION: usize = LONGENCODING_OFFSET_BYTES + 1; /* long offset + run encoding */

static ROOTS: AtomicUsize = AtomicUsize::new(0);
static REFS: AtomicUsize = AtomicUsize::new(0);

pub fn scan_boot_image<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
    unsafe {
        let boot_record = (JTOC_BASE + THE_BOOT_RECORD_FIELD_OFFSET).load::<Address>();
        let map_start = (boot_record + BOOT_IMAGE_R_MAP_START_OFFSET).load::<Address>();
        let map_end = ((boot_record + BOOT_IMAGE_R_MAP_END_OFFSET).load::<Address>());
        let image_start = (boot_record + BOOT_IMAGE_DATA_START_FIELD_OFFSET).load::<Address>();

        let collector = VMActivePlan::collector(tls);

        let stride = collector.parallel_worker_count() << LOG_CHUNK_BYTES;
        trace!("stride={}", stride);
        let start = collector.parallel_worker_ordinal() << LOG_CHUNK_BYTES;
        trace!("start={}", start);
        let mut cursor = map_start + start;
        trace!("cursor={:x}", cursor);

        ROOTS.store(0, Ordering::Relaxed);
        ROOTS.store(0, Ordering::Relaxed);

        while cursor < map_end {
            trace!("Processing chunk at {:x}", cursor);
            process_chunk(cursor, image_start, map_start, map_end, trace);
            trace!("Chunk processed successfully");
            cursor += stride;
        }
    }
}

fn process_chunk<T: TraceLocal>(chunk_start: Address, image_start: Address,
                                map_start: Address, map_end: Address, trace: &mut T) {
    let mut value: usize;
    let mut offset: usize = 0;
    let mut cursor: Address = chunk_start;
    unsafe {
        while { value = cursor.load::<u8>() as usize; value != 0 } {
            /* establish the offset */
            if (value & LONGENCODING_MASK) != 0 {
                offset = decode_long_encoding(cursor);
                cursor += LONGENCODING_OFFSET_BYTES;
            } else {
                offset += value & 0xfc;
                cursor += 1isize;
            }
            /* figure out the length of the run, if any */
            let mut runlength: usize = 0;
            if (value & RUN_MASK) != 0 {
                runlength = cursor.load::<u8>() as usize;
                cursor += 1isize;
            }
            /* enqueue the specified slot or slots */
            debug_assert!(conversions::is_address_aligned(Address::from_usize(offset)));
            let mut slot: Address = image_start + offset;
            if cfg!(feature = "debug") {
                REFS.fetch_add(1, Ordering::Relaxed);
            }

            if !FILTER || slot.load::<Address>() > map_end {
                if cfg!(feature = "debug") {
                    ROOTS.fetch_add(1, Ordering::Relaxed);
                }
                trace.process_root_edge(slot, false);
            }
            if runlength != 0 {
                for i in 0..runlength {
                    offset += BYTES_IN_ADDRESS;
                    slot = image_start + offset;
                    debug_assert!(conversions::is_address_aligned(slot));
                    if cfg!(feature = "debug") {
                        REFS.fetch_add(1, Ordering::Relaxed);
                    }
                    if !FILTER || slot.load::<Address>() > map_end {
                        if cfg!(feature = "debug") {
                            ROOTS.fetch_add(1, Ordering::Relaxed);
                        }
                        // TODO: check_reference(slot) ?
                        trace.process_root_edge(slot, false);
                    }
                }
            }
        }
    }
}

fn decode_long_encoding(cursor: Address) -> usize {
    unsafe {
        let mut value: usize;
        value = cursor.load::<u8>() as usize & 0x000000fc;
        value |= (((cursor + 1isize).load::<u8>() as usize) << BITS_IN_BYTE) & 0x0000ff00;
        value |= (((cursor + 2isize).load::<u8>() as usize) << (2 * BITS_IN_BYTE)) & 0x00ff0000;
        value |= (((cursor + 3isize).load::<u8>() as usize) << (3 * BITS_IN_BYTE)) & 0xff000000;
        value
    }
}
