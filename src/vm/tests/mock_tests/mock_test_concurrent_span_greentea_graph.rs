// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Green Tea span scanning in real concurrent cycles: a random object graph that mixes small
//! objects (span queue), larger `SpanSpace` objects (object queue, including multi-page spans)
//! and large objects in the LOS.  Several GC workers mark it while the mutator changes it through
//! the barrier.  Every object that is reachable at the end of a cycle must be live, and every
//! object that was unreachable when the cycle started must be dead.  Two cycles run, so the second
//! one starts from spans whose scan bits were cleared by sweeping.
//!
//! The test also passes with `MMTK_CONCURRENT_SPAN_GREEN_TEA=false`; it then only checks the
//! Green Tea counters when Green Tea is on.

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::plan::concurrent::span::greentea::stats;
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::ConcurrentPlan;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicUsize, Ordering};

const NUM_ROOTS: usize = 64;
const NUM_OBJECTS: usize = 30_000;
const MAX_FIELDS: usize = 4;
const LOS_SIZE: usize = 40 * 1024;

/// Root slots.  Each holds an object reference as a raw address, or 0 for null.
static ROOTS: [AtomicUsize; NUM_ROOTS] = [const { AtomicUsize::new(0) }; NUM_ROOTS];

/// The object header, at the object reference: `size << 8 | number of fields`.  The fields
/// follow it.
fn header(object: ObjectReference) -> Address {
    object.to_raw_address()
}

fn field(object: ObjectReference, i: usize) -> Address {
    object.to_raw_address() + (8 + i * 8)
}

fn nfields_for(size: usize) -> usize {
    // The object starts one word before the reference (MockVM), then the header.
    usize::min((size - 16) / 8, MAX_FIELDS)
}

fn object_size(object: ObjectReference) -> usize {
    unsafe { header(object).load::<usize>() >> 8 }
}

fn nfields(object: ObjectReference) -> usize {
    unsafe { header(object).load::<usize>() & 0xff }
}

/// A small deterministic random number generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Mostly small objects (span queue: 16-512 B), some larger ones (object queue: 1-2 KiB, and
/// 6 KiB in 3-page spans).  Large objects are added separately.
fn random_size(rng: &mut Rng) -> usize {
    const SIZES: [usize; 12] = [16, 16, 24, 32, 48, 48, 64, 128, 256, 512, 1024, 2048];
    if rng.below(100) == 0 {
        6144
    } else {
        SIZES[rng.below(SIZES.len())]
    }
}

/// The test's model of the heap: objects and their fields (indices into `objects`).
struct Graph {
    objects: Vec<ObjectReference>,
    fields: Vec<Vec<Option<usize>>>,
    roots: [Option<usize>; NUM_ROOTS],
}

impl Graph {
    fn alloc(&mut self, fixture: &MutatorFixture, size: usize) -> usize {
        let semantics = if size > crate::policy::spanspace::MAX_OBJECT_SIZE {
            AllocationSemantics::Los
        } else {
            AllocationSemantics::Default
        };
        let addr = memory_manager::alloc(fixture.mutator(), size, 8, 0, semantics);
        assert!(!addr.is_zero());
        let object = MockVM::object_start_to_ref(addr);
        memory_manager::post_alloc(fixture.mutator(), object, size, semantics);
        let n = nfields_for(size);
        unsafe { header(object).store::<usize>(size << 8 | n) };
        for i in 0..n {
            unsafe { field(object, i).store::<usize>(0) };
        }
        self.objects.push(object);
        self.fields.push(vec![None; n]);
        self.objects.len() - 1
    }

    /// Store without a barrier (no marking in progress).
    fn link(&mut self, from: usize, i: usize, to: Option<usize>) {
        let value = to.map_or(0, |t| self.objects[t].to_raw_address().as_usize());
        unsafe { field(self.objects[from], i).store::<usize>(value) };
        self.fields[from][i] = to;
    }

    /// Store with the write barrier (marking may be in progress).
    fn write(&mut self, fixture: &MutatorFixture, from: usize, i: usize, to: Option<usize>) {
        let src = self.objects[from];
        let target = to.map(|t| self.objects[t]);
        memory_manager::object_reference_write_pre(fixture.mutator(), src, field(src, i), target);
        self.link(from, i, to);
    }

    fn set_root(&mut self, r: usize, to: Option<usize>) {
        let value = to.map_or(0, |t| self.objects[t].to_raw_address().as_usize());
        ROOTS[r].store(value, Ordering::SeqCst);
        self.roots[r] = to;
    }

    fn reachable(&self) -> Vec<bool> {
        let mut seen = vec![false; self.objects.len()];
        let mut stack: Vec<usize> = self.roots.iter().flatten().copied().collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            stack.extend(self.fields[i].iter().flatten().copied());
        }
        seen
    }

    /// Fill every field with a random object (or null).
    fn randomize(&mut self, rng: &mut Rng, first: usize) {
        let n = self.objects.len();
        for from in first..n {
            for i in 0..self.fields[from].len() {
                let to = if rng.below(10) == 0 {
                    None
                } else {
                    Some(rng.below(n))
                };
                self.link(from, i, to);
            }
        }
    }
}

fn plan(fixture: &MutatorFixture) -> &'static ConcurrentSpan<MockVM> {
    fixture
        .mmtk()
        .get_plan()
        .downcast_ref::<ConcurrentSpan<MockVM>>()
        .unwrap()
}

/// Run one concurrent cycle.  While marking, the mutator moves `mutations` random edges to
/// objects that were reachable when the cycle started.  Returns the objects that were reachable
/// at the start and at the end.
fn run_cycle(
    fixture: &MutatorFixture,
    graph: &mut Graph,
    rng: &mut Rng,
    mutations: usize,
) -> (Vec<bool>, Vec<bool>) {
    let mmtk = fixture.mmtk();
    let tls = fixture.mutator_tls();
    let plan = plan(fixture);
    let at_start = graph.reachable();
    let reachable: Vec<usize> = (0..graph.objects.len()).filter(|i| at_start[*i]).collect();

    assert!(mmtk.gc_trigger.poll(true, None));
    MockVM::block_for_gc(tls);
    if !plan.concurrent_work_in_progress() {
        warn!("Marking finished before the mutator ran");
    }

    // Mutate while marking.
    for _ in 0..mutations {
        if mmtk.state.gc_status.load() == GcStatus::PauseRequested {
            MockVM::block_for_gc(tls);
        }
        let from = reachable[rng.below(reachable.len())];
        if graph.fields[from].is_empty() {
            continue;
        }
        let i = rng.below(graph.fields[from].len());
        let to = if rng.below(4) == 0 {
            None
        } else {
            Some(reachable[rng.below(reachable.len())])
        };
        graph.write(fixture, from, i, to);
    }

    // Park whenever a pause is requested, until the cycle is over.
    let mut finished = false;
    for _ in 0..100_000 {
        match mmtk.state.gc_status.load() {
            GcStatus::PauseRequested => MockVM::block_for_gc(tls),
            GcStatus::NotInGC if !plan.concurrent_work_in_progress() => {
                finished = true;
                break;
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
    assert!(finished, "The concurrent cycle did not finish");
    (at_start, graph.reachable())
}

fn check(graph: &Graph, at_start: &[bool], at_end: &[bool]) {
    let mut live = 0;
    let mut dead = 0;
    for (i, object) in graph.objects.iter().enumerate() {
        if at_end[i] {
            assert!(
                memory_manager::is_live_object(*object),
                "object {i} ({object}, {} bytes) is reachable but dead",
                object_size(*object)
            );
            live += 1;
        } else if !at_start[i] {
            assert!(
                !memory_manager::is_live_object(*object),
                "object {i} ({object}, {} bytes) was unreachable but is live",
                object_size(*object)
            );
            dead += 1;
        }
    }
    info!("checked {live} live and {dead} dead objects");
    assert!(live > NUM_OBJECTS / 2 && dead > 100);
}

#[test]
pub fn green_tea_graph() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| {
                    ROOTS.iter().map(Address::from_ref).collect()
                })),
                get_object_size: MockMethod::new_fixed(Box::new(object_size)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    // Slow down marking a little, so that the mutator runs while marking.
                    if (object.to_raw_address().as_usize() >> 4) % 16 == 0 {
                        std::thread::sleep(std::time::Duration::from_micros(50));
                    }
                    for i in 0..nfields(object) {
                        visitor.visit_slot(field(object, i));
                    }
                })),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_builder(|builder| {
                builder.options.gc_trigger.set(
                    crate::util::options::GCTriggerSelector::FixedHeapSize(64 * MB),
                );
                // Only the test starts cycles (Go's `GOGC=off`): objects are unreachable until
                // the graph is built.
                builder.options.gc_percent.set(-1);
                // Several marking workers, so spans are marked and scanned by different workers.
                builder.options.threads.set(4);
                builder.options.concurrent_threads.set(4);
            });
            let green_tea = *fixture.mmtk().get_options().concurrent_span_green_tea;
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
            let mut graph = Graph {
                objects: vec![],
                fields: vec![],
                roots: [None; NUM_ROOTS],
            };

            for _ in 0..NUM_OBJECTS {
                let size = random_size(&mut rng);
                graph.alloc(&fixture, size);
            }
            // Large objects, each rooted.
            for r in 0..16 {
                let los = graph.alloc(&fixture, LOS_SIZE);
                graph.set_root(r, Some(los));
            }
            graph.randomize(&mut rng, 0);
            for r in 16..NUM_ROOTS {
                let to = rng.below(NUM_OBJECTS);
                graph.set_root(r, Some(to));
            }

            let (at_start, at_end) = run_cycle(&fixture, &mut graph, &mut rng, 5_000);
            check(&graph, &at_start, &at_end);

            // A second cycle, after sweeping: some garbage, new objects and new edges.
            let first_new = graph.objects.len();
            for _ in 0..NUM_OBJECTS / 4 {
                let size = random_size(&mut rng);
                graph.alloc(&fixture, size);
            }
            // Only live objects may point to the new ones; the dead ones' memory may be reused.
            let n = graph.objects.len();
            for from in first_new..n {
                for i in 0..graph.fields[from].len() {
                    let to = loop {
                        let to = rng.below(n);
                        if to >= first_new || at_end[to] {
                            break to;
                        }
                    };
                    graph.link(from, i, Some(to));
                }
            }
            let live: Vec<usize> = (0..first_new).filter(|i| at_end[*i]).collect();
            for _ in 0..NUM_OBJECTS / 4 {
                let from = live[rng.below(live.len())];
                if !graph.fields[from].is_empty() {
                    let i = rng.below(graph.fields[from].len());
                    graph.link(from, i, Some(first_new + rng.below(n - first_new)));
                }
            }
            // Forget the dead objects: their cells may hold new objects now.
            let alive: Vec<bool> = (0..n).map(|i| i >= first_new || at_end[i]).collect();
            for (i, fields) in graph.fields.iter_mut().enumerate() {
                if !alive[i] {
                    fields.clear();
                }
            }
            let (at_start2, at_end2) = run_cycle(&fixture, &mut graph, &mut rng, 5_000);
            let unreachable_live_before = (0..n).filter(|i| !alive[*i]).count();
            let at_start2: Vec<bool> = (0..n).map(|i| at_start2[i] || !alive[i]).collect();
            info!("{unreachable_live_before} objects were freed by the first cycle");
            check(&graph, &at_start2, &at_end2);

            let spans = stats::SPANS_SCANNED.load(Ordering::Relaxed);
            let batched = stats::SPAN_SCANS_BATCHED.load(Ordering::Relaxed);
            let objects = stats::SPAN_OBJECTS_SCANNED.load(Ordering::Relaxed);
            let spilled = stats::SPANS_SPILLED.load(Ordering::Relaxed);
            info!("span scans: {spans} ({batched} batched, {objects} objects, {spilled} spilled)");
            if green_tea {
                assert!(spans > 0, "No span was scanned");
                assert!(batched > 0, "No span scan scanned more than one object");
                assert!(objects > spans);
            } else {
                assert_eq!(spans, 0);
            }
        },
        no_cleanup,
    )
}
