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
    static ref GLOBAL_RS_BUFFER: Mutex<Vec<Vec<Card>>> = Mutex::new(vec![]);
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
    fn refine_one_buffer(&self, buf: Vec<Card>) {
        for card in buf {
            refine_one_card(card);
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
                REQUEST_FLAG.store(true, Ordering::Relaxed);
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

fn refine_one_card(card: Card) {
    if card.get_state() != CardState::Dirty {
        return;
    }
    card.set_state(CardState::NotDirty);
    
    let current_region = card.get_region();
    card.linear_scan(|obj| {
        debug_assert!(VMObjectModel::object_start_ref(obj) >= card.0, "card {:?}, obj {:?}: {:?}..{:?}", card.0, obj, VMObjectModel::object_start_ref(obj), VMObjectModel::get_object_end_address(obj));
        debug_assert!(VMObjectModel::object_start_ref(obj) < card.0 + BYTES_IN_CARD, "card {:?}, obj {:?}: {:?}..{:?}", card.0, obj, VMObjectModel::object_start_ref(obj), VMObjectModel::get_object_end_address(obj));
        
        scan_edge(obj, |slot| {
            let field = unsafe { slot.load::<ObjectReference>() };
            if !field.is_null() {
                if PLAN.region_space.in_space(field) {
                    let other_region = Region::of_object(field);
                    if other_region.0 != current_region {
                        other_region.remset.add_card(card)
                    }
                }
            }
        });
    });
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
    if REQUEST_FLAG.load(Ordering::Relaxed) {
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

pub fn collector_refine_all_dirty_cards() {
    for i in 0..CARDS_IN_HEAP {
        let card = Card(HEAP_START + (i << LOG_BYTES_IN_CARD));
        if card.get_state() == CardState::Dirty {
            refine_one_card(card);
        }
    }
}

pub fn enquene(buf: Vec<Card>) {
    if ABORT.load(Ordering::SeqCst) {
        return;
    }
    let mut global_buffer = GLOBAL_RS_BUFFER.lock().unwrap();
    global_buffer.push(buf);
    if global_buffer.len() > 5 {
        trigger_concurrent_refine();
    }
}
