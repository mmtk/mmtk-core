use std::sync::{Mutex, Condvar};
use std::sync::atomic::{AtomicBool, Ordering};
use policy::region::*;
use vm::*;
use util::*;
use super::PLAN;
use super::*;
use policy::space::Space;
use util::heap::layout::vm_layout_constants::*;

lazy_static! {
    static ref GLOBAL_RS_BUFFER: Mutex<Vec<Box<Vec<Card>>>> = Mutex::new(vec![]);
    static ref CVAR: Condvar = Condvar::new();
    static ref SYNC: Mutex<ConcurrentRefineWorkerGroupSync> = Mutex::new(ConcurrentRefineWorkerGroupSync {
        trigger_count: 1,
        contexts_parked: 0,
    });
    static ref ABORT: AtomicBool = AtomicBool::new(false);
    static ref REQUEST_FLAG: AtomicBool = AtomicBool::new(false);
}

struct ConcurrentRefineWorkerGroupSync {
    trigger_count: usize,
    contexts_parked: usize,
}

struct ConcurrentRefineWorker {
    #[allow(dead_code)] id: usize,
    last_trigger_count: usize,
}

impl ConcurrentRefineWorker {
    fn refine_one_buffer(&self, buf: Box<Vec<Card>>) {
        for card in *buf {
            if ENABLE_HOT_CARDS_OPTIMIZATION && card.inc_hotness() {
                // Skip this hot card
                continue;
            }
            refine_one_card(card, false);
            if ABORT.load(Ordering::SeqCst) {
                return;
            }
        }
    }
    
    fn refine(&self) {
        if !ABORT.load(Ordering::SeqCst) {
            while let Some(buf) = { GLOBAL_RS_BUFFER.lock().unwrap().pop() } {
                self.refine_one_buffer(buf);
                if ABORT.load(Ordering::SeqCst) {
                    return;
                }
            }
        }
    }

    fn run(&mut self) {
        loop {
            self.park();
            self.refine();
        }
    }

    fn park(&mut self) {
        let mut inner = SYNC.lock().unwrap();
        self.last_trigger_count += 1;
        if self.last_trigger_count == inner.trigger_count {
            inner.contexts_parked += 1;
            if inner.contexts_parked == CONCURRENT_REFINEMENT_THREADS {
                // ABORT.store(false, Ordering::Relaxed);
                REQUEST_FLAG.store(false, Ordering::Relaxed);
            }
            CVAR.notify_all();
            while self.last_trigger_count == inner.trigger_count {
                inner = CVAR.wait(inner).unwrap();
            }
        }
    }
}


pub fn scan_edge<F: Fn(Address)>(object: ObjectReference, f: F) {
    struct ObjectFieldsClosure<F: Fn(Address)>(F);
    impl <F: Fn(Address)> ::plan::TransitiveClosure for ObjectFieldsClosure<F> {
        #[inline(always)]
        fn process_edge(&mut self, _src: ObjectReference, slot: Address) {
            (self.0)(slot)
        }
        fn process_node(&mut self, _object: ObjectReference) {
            unreachable!();
        }
    }
    let mut closure = ObjectFieldsClosure(f);
    VMScanning::scan_object(&mut closure, object, 0 as _);
}

fn refine_one_card(card: Card, mark_dead: bool) -> bool {
    if card.get_state() != CardState::Dirty {
        return false;
    }
    card.set_state(CardState::NotDirty);
    card.linear_scan(|obj| {
        debug_assert!(VMObjectModel::object_start_ref(obj) >= card.start(), "card {:?}, obj {:?}: {:?}..{:?}", card.start(), obj, VMObjectModel::object_start_ref(obj), VMObjectModel::get_object_end_address(obj));
        debug_assert!(VMObjectModel::object_start_ref(obj) < card.start() + BYTES_IN_CARD, "card {:?}, obj {:?}: {:?}..{:?}", card.start(), obj, VMObjectModel::object_start_ref(obj), VMObjectModel::get_object_end_address(obj));
        
        scan_edge(obj, |slot| {
            let field = unsafe { slot.load::<ObjectReference>() };
            // obj.slot -> field
            if RegionSpace::is_cross_region_ref(obj, slot, field) && PLAN.region_space.in_space(field) {
                Region::of_object(field).remset().add_card(card)
            }
        });
    }, mark_dead);
    true
}

pub fn spawn_refine_threads() {
    for id in 0..CONCURRENT_REFINEMENT_THREADS {
        ::std::thread::spawn(move || {
            let mut worker = ConcurrentRefineWorker { id, last_trigger_count: 0 };
            worker.run();
        });
    }
}

fn trigger_concurrent_refine() {
    if REQUEST_FLAG.load(Ordering::Relaxed) || ABORT.load(Ordering::Relaxed) {
        return
    }
    let mut inner = SYNC.lock().unwrap();
    if !REQUEST_FLAG.load(Ordering::Relaxed) {
        REQUEST_FLAG.store(true, Ordering::Relaxed);
        inner.trigger_count += 1;
        inner.contexts_parked = 0;
        CVAR.notify_all();
    }
}

pub fn disable_concurrent_refinement() {
    ABORT.store(true, Ordering::SeqCst);
}

pub fn enable_concurrent_refinement() {
    ABORT.store(false, Ordering::SeqCst);
}

pub fn collector_refine_all_dirty_cards(id: usize, num_workers: usize) {
    if id == 0 {
        let mut global_buffer = GLOBAL_RS_BUFFER.lock().unwrap();
        global_buffer.clear();
    }
    let start_time = ::std::time::SystemTime::now();
    let size = (CARDS_IN_HEAP + num_workers - 1) / num_workers;
    let start = size * id;
    let limit = size * (id + 1);
    let limit = if limit > CARDS_IN_HEAP { CARDS_IN_HEAP } else { limit };
    let mut cards = 0;
    for i in start..limit {
        let card = unsafe { Card::unchecked(HEAP_START + (i << LOG_BYTES_IN_CARD)) };
        if card.get_state() == CardState::Dirty {
            if refine_one_card(card, false) {
                cards += 1;
            }
        }
    }
    let time = start_time.elapsed().unwrap().as_millis() as usize;
    PLAN.predictor.timer.report_dirty_card_scanning_time(time, cards);
}

pub fn collector_clear_hotness_table(id: usize, num_workers: usize) {
    CardTable::clear_all_hotness_par(id, num_workers)
}

pub fn enquene(buf: Box<Vec<Card>>) {
    if ABORT.load(Ordering::SeqCst) {
        return;
    }
    let mut global_buffer = GLOBAL_RS_BUFFER.lock().unwrap();
    global_buffer.push(buf);
    if global_buffer.len() > 5 {
        trigger_concurrent_refine();
    }
}
