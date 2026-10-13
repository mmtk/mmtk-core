// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! `AllocatorInfo::Span` (design §5.3 "OpenJDK fast path"): a binding's inline fast path,
//! emulated here with raw loads and stores at the offsets that `AllocatorInfo` gives, allocates
//! the same objects as `SpanAllocator::alloc`, allocates black while the space does (marking the
//! object, setting its scan bit in span-queue spans and counting its bytes), and mixes with the
//! slow path.

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::plan::AllocationSemantics;
use crate::policy::space::Space;
use crate::util::alloc::span_allocator::span_class_index;
use crate::util::alloc::AllocatorInfo;
use crate::util::Address;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

struct Info {
    spans: usize,
    space: usize,
    black: usize,
    inline_black: usize,
    black_bytes: usize,
    mark_bits: usize,
    scan_bits: usize,
    black_scan: usize,
    alloc_cache: usize,
    freeindex: usize,
    nelems: usize,
    elemsize: usize,
    start: usize,
    alloc_count: usize,
}

/// Go's `nextFreeFast` as a binding emits it. `None` means "take the slow path".
unsafe fn inline_alloc(mutator: Address, info: &Info, size: usize) -> Option<Address> {
    let space = Address::from_usize((mutator + info.space).load::<usize>());
    let black = (space + info.black).load::<u8>() != 0;
    if black && (space + info.inline_black).load::<u8>() == 0 {
        return None;
    }
    let spc = span_class_index(size, 8, false)?;
    let span = (mutator + info.spans + spc * 8).load::<Address>();
    let cache = (span + info.alloc_cache).load::<u64>();
    if cache == 0 {
        return None;
    }
    let bit = cache.trailing_zeros() as usize;
    let result = (span + info.freeindex).load::<usize>() + bit;
    let nelems = (span + info.nelems).load::<usize>();
    if result >= nelems {
        return None;
    }
    let freeidx = result + 1;
    if freeidx % 64 == 0 && freeidx != nelems {
        return None;
    }
    // Two shifts, as in generated code: a shift by 64 must give 0.
    (span + info.alloc_cache).store::<u64>((cache >> bit) >> 1);
    (span + info.freeindex).store::<usize>(freeidx);
    let count = (span + info.alloc_count).load::<usize>();
    (span + info.alloc_count).store::<usize>(count + 1);
    let elemsize = (span + info.elemsize).load::<usize>();
    if black {
        // Atomic ORs: GC threads set other bits of the same words.
        let set_bit = |bitmap: usize| {
            let word = &*((span + bitmap + (result / 64) * 8).to_ptr::<AtomicU64>());
            let old = word.fetch_or(1 << (result % 64), Ordering::SeqCst);
            assert_eq!(old & (1 << (result % 64)), 0, "bit {result} already set");
        };
        if (span + info.black_scan).load::<u8>() != 0 {
            set_bit(info.scan_bits);
        }
        set_bit(info.mark_bits);
        let bytes = (mutator + info.black_bytes).load::<usize>();
        (mutator + info.black_bytes).store::<usize>(bytes + elemsize);
    }
    let start = (span + info.start).load::<Address>();
    Some(start + result * elemsize)
}

#[test]
pub fn inline_fastpath() {
    with_mockvm(
        default_setup,
        || {
            let fixture = MutatorFixture::create_with_heapsize(64 << 20);
            let Some(space) = span_space(fixture.mmtk()) else {
                eprintln!("No SpanSpace in this plan: test skipped");
                return;
            };
            let selector =
                memory_manager::get_allocator_mapping(fixture.mmtk(), AllocationSemantics::Default);
            let AllocatorInfo::Span {
                spans_offset,
                space_offset,
                allocate_black_offset,
                inline_black_offset,
                black_bytes_offset,
                alloc_cache_offset,
                freeindex_offset,
                nelems_offset,
                elemsize_offset,
                start_offset,
                alloc_count_offset,
                needzero_offset: _,
                mark_bits_offset,
                scan_bits_offset,
                black_scan_offset,
            } = AllocatorInfo::new::<MockVM>(selector)
            else {
                panic!("expected AllocatorInfo::Span");
            };
            let info = Info {
                spans: spans_offset,
                space: space_offset,
                black: allocate_black_offset,
                inline_black: inline_black_offset,
                black_bytes: black_bytes_offset,
                mark_bits: mark_bits_offset,
                scan_bits: scan_bits_offset,
                black_scan: black_scan_offset,
                alloc_cache: alloc_cache_offset,
                freeindex: freeindex_offset,
                nelems: nelems_offset,
                elemsize: elemsize_offset,
                start: start_offset,
                alloc_count: alloc_count_offset,
            };
            let mutator = fixture.mutator();
            let mutator_addr = Address::from_mut_ptr(mutator as *mut _);

            let sizes = [16usize, 24, 40, 64, 200, 1000, 3000, 16384];
            let mut seen = HashSet::new();
            let mut fast = 0;
            let mut sweepgen = space.sweepgen();
            for round in 0..300 {
                for &size in sizes.iter() {
                    // Compiled code does not call `post_alloc` after the inline fast path.
                    let addr = match unsafe { inline_alloc(mutator_addr, &info, size) } {
                        Some(a) => {
                            fast += 1;
                            a
                        }
                        None => {
                            let a = memory_manager::alloc(
                                mutator,
                                size,
                                8,
                                0,
                                AllocationSemantics::Default,
                            );
                            let obj = MockVM::object_start_to_ref(a);
                            memory_manager::post_alloc(
                                mutator,
                                obj,
                                size,
                                AllocationSemantics::Default,
                            );
                            a
                        }
                    };
                    assert!(!addr.is_zero());
                    if space.sweepgen() != sweepgen {
                        // A GC (triggered by the pacer) freed the unreachable objects.
                        sweepgen = space.sweepgen();
                        seen.clear();
                    }
                    assert!(
                        seen.insert(addr),
                        "{addr} allocated twice (size {size}, round {round})"
                    );
                    let obj = MockVM::object_start_to_ref(addr);
                    let span = space.span_of(addr).expect("not in a span");
                    assert!(span.elemsize() >= size);
                    assert_eq!(span.object_address(span.obj_index(addr)), addr);
                    fill(obj, size, 0x5a);
                }
            }
            assert!(fast > 0, "the inline fast path never succeeded");

            // While marking, the inline fast path allocates black (unless the space needs log
            // bits for new objects, as with the SATB barrier). Run a real cycle: `FinalMark`
            // counts the bytes, and in debug builds checks them against the mark bits.
            let plan = fixture.mmtk().get_plan().concurrent().unwrap();
            let tls = fixture.mutator_tls();
            let wait_for_no_cycle = || {
                for _ in 0..10_000 {
                    match fixture.mmtk().state.gc_status.load() {
                        GcStatus::PauseRequested => MockVM::block_for_gc(tls),
                        GcStatus::NotInGC if !plan.concurrent_work_in_progress() => return,
                        _ => std::thread::sleep(std::time::Duration::from_millis(1)),
                    }
                }
                panic!("The concurrent cycle did not finish");
            };
            wait_for_no_cycle();
            assert!(fixture.mmtk().gc_trigger.poll(true, None));
            MockVM::block_for_gc(tls);
            let inline_black = unsafe {
                let space_ptr = Address::from_usize((mutator_addr + info.space).load::<usize>());
                (space_ptr + info.inline_black).load::<u8>() != 0
            };
            let black_bytes = || unsafe { (mutator_addr + info.black_bytes).load::<usize>() };
            let mut black = 0;
            let mut scanned = 0;
            for _ in 0..50 {
                for &size in sizes.iter() {
                    let allocating_black =
                        plan.concurrent_work_in_progress() && space.should_allocate_as_live();
                    let before = black_bytes();
                    let Some(addr) = (unsafe { inline_alloc(mutator_addr, &info, size) }) else {
                        // A refill, or no inline black allocation: the slow path allocates
                        // (black while marking), and may stop for a pause.
                        let obj = alloc_object(mutator, size);
                        if allocating_black && space.should_allocate_as_live() {
                            let addr = object_start(obj);
                            let span = space.span_of(addr).unwrap();
                            assert!(span.is_marked(span.obj_index(addr)));
                        }
                        continue;
                    };
                    let span = space.span_of(addr).expect("not in a span");
                    let index = span.obj_index(addr);
                    if !allocating_black {
                        assert!(!span.is_marked(index));
                        continue;
                    }
                    assert!(
                        inline_black,
                        "the inline fast path allocated black without permission"
                    );
                    black += 1;
                    assert!(
                        span.is_marked(index),
                        "{addr} allocated black but not marked"
                    );
                    if space.green_tea() && span.spanclass().uses_span_queue() {
                        assert!(
                            span.is_scanned(index),
                            "{addr} allocated black without its scan bit"
                        );
                        scanned += 1;
                    }
                    assert_eq!(black_bytes(), before + span.elemsize());
                }
            }
            if inline_black {
                assert!(black > 0, "the inline fast path never allocated black");
                assert!(
                    !space.green_tea() || scanned > 0,
                    "no span-queue object allocated black"
                );
            }
            wait_for_no_cycle();
            assert_eq!(black_bytes(), 0, "FinalMark did not count the black bytes");
        },
        no_cleanup,
    )
}
