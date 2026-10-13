// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Reusing partially live spans: some small objects survive a concurrent cycle (per size class:
//! all but the last of every 64, or every other one), then the mutator allocates into the freed
//! cells.  New objects must be zeroed and must not overlap the
//! survivors.  This exercises allocation from swept alloc bits across 64-object boundaries.

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use std::sync::Mutex;

const N: usize = 20_000;
const SIZES: [usize; 4] = [16, 24, 48, 128];

lazy_static! {
    /// Root slots: raw object addresses, 0 for null.
    static ref ROOTS: Mutex<Vec<usize>> = Mutex::new(vec![0; N]);
}

fn root_slots() -> Vec<Address> {
    let roots = ROOTS.lock().unwrap();
    (0..N).map(|i| Address::from_ref(&roots[i])).collect()
}

fn pattern(i: usize) -> u8 {
    (i % 251 + 1) as u8
}

fn run_cycle(fixture: &MutatorFixture) {
    let mmtk = fixture.mmtk();
    let tls = fixture.mutator_tls();
    assert!(mmtk.gc_trigger.poll(true, None));
    MockVM::block_for_gc(tls);
    for _ in 0..20_000 {
        match mmtk.state.gc_status.load() {
            GcStatus::PauseRequested => MockVM::block_for_gc(tls),
            GcStatus::NotInGC
                if !mmtk
                    .get_plan()
                    .concurrent()
                    .unwrap()
                    .concurrent_work_in_progress() =>
            {
                return
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
    panic!("The concurrent cycle did not finish");
}

#[test]
#[allow(clippy::needless_range_loop)]
pub fn reuse_partially_live_spans() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| root_slots())),
                get_object_size: MockMethod::new_fixed(Box::new(|_| 16)),
                // Objects have no reference fields.
                scan_object: MockMethod::new_fixed(Box::new(|_| {})),
                ..MockVM::default()
            }
        },
        || {
            let fixture = MutatorFixture::create_with_heapsize(64 << 20);
            if span_space(fixture.mmtk()).is_none() {
                return;
            }
            let mutator = fixture.mutator();
            // The survivor held by each root slot: (object, size, fill byte).
            let mut live: Vec<Option<(ObjectReference, usize, u8)>> = vec![None; N];
            for round in 0..3 {
                let mut class_count = [0usize; SIZES.len()];
                for i in 0..N {
                    let class = i % SIZES.len();
                    let size = SIZES[class];
                    let c = class_count[class];
                    class_count[class] += 1;
                    // Free only the last of every 64 objects of a class (the last bit of an
                    // alloc-cache word), or every other one.
                    let keep = if round == 1 { c % 2 == 0 } else { c % 64 != 63 };
                    let obj = alloc_object(mutator, size);
                    assert_zeroed(obj, size);
                    let p = pattern(i + round);
                    fill(obj, size, p);
                    if keep {
                        ROOTS.lock().unwrap()[i] = obj.to_raw_address().as_usize();
                        live[i] = Some((obj, size, p));
                    }
                }
                run_cycle(&fixture);
                for (obj, size, p) in live.iter().flatten() {
                    assert!(memory_manager::is_live_object(*obj));
                    check_filled(*obj, *size, *p);
                }
            }
        },
        no_cleanup,
    )
}
