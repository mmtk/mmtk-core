// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Several mutator threads build and rewire linked lists through the write barrier while
//! concurrent cycles run (design §10.2).  Every object carries a payload, and every thread
//! checks the payload of the objects it can reach, so an object freed while still reachable (and
//! reused) shows up as corrupted data.
//!
//! MockVM's concurrent stack scanning and mutator flushes touch other threads' mutators without
//! stopping them, so this test scans stacks in `InitialMark` and lets `FinalMark` check the
//! barrier buffers.  Allocate-black, the barrier, sweeping, assists and stalls all run
//! concurrently with the mutators.

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::util::test_util::mock_vm::mock_api;
use crate::util::{Address, ObjectReference, VMMutatorThread};
use crate::vm::Collection;
use crate::AllocationSemantics;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const THREADS: usize = 4;
const ROOTS_PER_THREAD: usize = 64;
const STEPS: usize = 60_000;
const OBJECT_SIZE: usize = 48;

lazy_static! {
    /// Each mutator's root slots (its "stack"), keyed by its tls.
    static ref ROOTS: Mutex<HashMap<usize, &'static [AtomicUsize]>> = Mutex::new(HashMap::new());
}
static WORKERS_DONE: AtomicUsize = AtomicUsize::new(0);

/// The reference field.
fn next(object: ObjectReference) -> Address {
    object.to_raw_address() + 8usize
}

/// Payload: three copies of the same non-zero word.
fn payload(object: ObjectReference, i: usize) -> Address {
    object.to_raw_address() + 16usize + i * 8
}

fn check(object: ObjectReference) {
    let v = unsafe { payload(object, 0).load::<usize>() };
    assert_ne!(v, 0, "object {object} lost its payload");
    for i in 1..3 {
        let w = unsafe { payload(object, i).load::<usize>() };
        assert_eq!(v, w, "object {object} was overwritten");
    }
}

/// A tiny xorshift generator, so the test needs no crate.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize
    }
}

fn safepoint(tls: VMMutatorThread) {
    if mock_api::singleton().state.gc_status.load() == GcStatus::PauseRequested {
        MockVM::block_for_gc(tls);
    }
}

fn mutator_thread(seed: u64) {
    let tls = mock_api::bind_mutator();
    let roots: &'static [AtomicUsize] = Box::leak(
        (0..ROOTS_PER_THREAD)
            .map(|_| AtomicUsize::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    ROOTS
        .lock()
        .unwrap()
        .insert(tls.0 .0.to_address().as_usize(), roots);
    let mutator = tls.as_mock_mutator();
    let mut rng = Rng(seed);
    let read = |slot: &AtomicUsize| {
        ObjectReference::from_raw_address(unsafe {
            Address::from_usize(slot.load(Ordering::SeqCst))
        })
    };

    for step in 0..STEPS {
        safepoint(tls);
        let r = rng.next() % ROOTS_PER_THREAD;
        match rng.next() % 4 {
            // Build a new list and put it in a root slot (a stack write, no barrier).
            0 | 1 => {
                let len = 1 + rng.next() % 20;
                let mut head: Option<ObjectReference> = None;
                for _ in 0..len {
                    let addr = memory_manager::alloc(
                        mutator,
                        OBJECT_SIZE,
                        8,
                        0,
                        AllocationSemantics::Default,
                    );
                    let obj = MockVM::object_start_to_ref(addr);
                    memory_manager::post_alloc(
                        mutator,
                        obj,
                        OBJECT_SIZE,
                        AllocationSemantics::Default,
                    );
                    let v = rng.next() | 1;
                    unsafe {
                        next(obj).store(0usize);
                        for i in 0..3 {
                            payload(obj, i).store(v);
                        }
                    }
                    if let Some(h) = head {
                        memory_manager::object_reference_write_pre(
                            mutator,
                            obj,
                            next(obj),
                            Some(h),
                        );
                        unsafe { next(obj).store(h) };
                    }
                    head = Some(obj);
                }
                roots[r].store(head.unwrap().to_raw_address().as_usize(), Ordering::SeqCst);
            }
            // Move the tail of one list to another (heap writes, with the barrier).
            2 => {
                let s = rng.next() % ROOTS_PER_THREAD;
                if let (Some(a), Some(b)) = (read(&roots[r]), read(&roots[s])) {
                    if a != b {
                        let tail = unsafe { next(a).load::<Option<ObjectReference>>() };
                        memory_manager::object_reference_write_pre(mutator, a, next(a), None);
                        unsafe { next(a).store(0usize) };
                        if let Some(t) = tail {
                            let old = unsafe { next(b).load::<Option<ObjectReference>>() };
                            memory_manager::object_reference_write_pre(
                                mutator,
                                b,
                                next(b),
                                Some(t),
                            );
                            unsafe { next(b).store(t) };
                            // Keep what `b` pointed to reachable from the root slot `s`.
                            if let Some(o) = old {
                                roots[s].store(o.to_raw_address().as_usize(), Ordering::SeqCst);
                            }
                        }
                    }
                }
            }
            // Walk and check a list.
            _ => {
                let mut cur = read(&roots[r]);
                let mut n = 0;
                while let Some(o) = cur {
                    check(o);
                    cur = unsafe { next(o).load::<Option<ObjectReference>>() };
                    n += 1;
                    assert!(n < 1_000_000, "cycle in a list");
                }
            }
        }
        if step % 1000 == 0 {
            // Drop a root now and then.
            roots[rng.next() % ROOTS_PER_THREAD].store(0, Ordering::SeqCst);
        }
    }
    WORKERS_DONE.fetch_add(1, Ordering::SeqCst);
    // Stay a well-behaved mutator until the process exits: a registered mutator that stops
    // polling would block every later pause, and with it anyone waiting for that pause.
    loop {
        safepoint(tls);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
pub fn concurrent_stress() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                mutator_root_slots: MockMethod::new_fixed(Box::new(|tls: VMMutatorThread| {
                    match ROOTS.lock().unwrap().get(&tls.0 .0.to_address().as_usize()) {
                        Some(roots) => roots.iter().map(Address::from_ref).collect(),
                        None => vec![],
                    }
                })),
                get_object_size: MockMethod::new_fixed(Box::new(|_| OBJECT_SIZE)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    visitor.visit_slot(next(object));
                })),
                supports_blocking_mutators: MockMethod::new_fixed(Box::new(|_| true)),
                ..MockVM::default()
            }
        },
        || {
            let fixture = MutatorFixture::create_with_heapsize(4 << 20);
            let tls = fixture.mutator_tls();
            // The threads are not joined: they keep acting as mutators until the process exits.
            for i in 0..THREADS {
                std::thread::spawn(move || mutator_thread(0x9E3779B97F4A7C15 ^ (i as u64 + 1)));
            }
            // The fixture's mutator must stop for pauses too.
            while WORKERS_DONE.load(Ordering::SeqCst) < THREADS {
                safepoint(tls);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let plan = fixture.mmtk().get_plan();
            info!(
                "Stress test done, {} pages reserved",
                plan.get_reserved_pages()
            );
        },
        no_cleanup,
    )
}
