// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! `AllocatorInfo::Span` (design §5.3 "OpenJDK fast path"): a binding's inline fast path,
//! emulated here with raw loads and stores at the offsets that `AllocatorInfo` gives, allocates
//! the same objects as `SpanAllocator::alloc`, sees the allocate-black flag, and mixes with the
//! slow path.

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;
use crate::plan::AllocationSemantics;
use crate::policy::space::Space;
use crate::util::alloc::span_allocator::span_class_index;
use crate::util::alloc::AllocatorInfo;
use crate::util::Address;
use std::collections::HashSet;

struct Info {
    spans: usize,
    space: usize,
    black: usize,
    alloc_cache: usize,
    freeindex: usize,
    nelems: usize,
    elemsize: usize,
    start: usize,
    alloc_count: usize,
}

/// Go's `nextFreeFast` as a binding emits it. `None` means "take the slow path".
unsafe fn inline_alloc(mutator: Address, info: &Info, size: usize) -> Option<Address> {
    let space = (mutator + info.space).load::<usize>();
    if Address::from_usize(space + info.black).load::<u8>() != 0 {
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
    let start = (span + info.start).load::<Address>();
    Some(start + result * (span + info.elemsize).load::<usize>())
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
                alloc_cache_offset,
                freeindex_offset,
                nelems_offset,
                elemsize_offset,
                start_offset,
                alloc_count_offset,
                needzero_offset: _,
            } = AllocatorInfo::new::<MockVM>(selector)
            else {
                panic!("expected AllocatorInfo::Span");
            };
            let info = Info {
                spans: spans_offset,
                space: space_offset,
                black: allocate_black_offset,
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
                    let addr = match unsafe { inline_alloc(mutator_addr, &info, size) } {
                        Some(a) => {
                            fast += 1;
                            a
                        }
                        None => {
                            memory_manager::alloc(mutator, size, 8, 0, AllocationSemantics::Default)
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
                    memory_manager::post_alloc(mutator, obj, size, AllocationSemantics::Default);
                    let span = space.span_of(addr).expect("not in a span");
                    assert!(span.elemsize() >= size);
                    assert_eq!(span.object_address(span.obj_index(addr)), addr);
                    fill(obj, size, 0x5a);
                }
            }
            assert!(fast > 0, "the inline fast path never succeeded");

            // While allocating black, the inline fast path must not allocate.
            space.prepare_initial_mark();
            space.set_allocate_as_live(true);
            for &size in sizes.iter() {
                assert!(unsafe { inline_alloc(mutator_addr, &info, size) }.is_none());
            }
            space.set_allocate_as_live(false);
            let flag = unsafe {
                let space_ptr = (mutator_addr + info.space).load::<usize>();
                Address::from_usize(space_ptr + info.black).load::<u8>()
            };
            assert_eq!(flag, 0);
        },
        no_cleanup,
    )
}
