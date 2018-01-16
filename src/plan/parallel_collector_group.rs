use std::vec::Vec;
use std::sync::{Mutex, Condvar};

use super::ParallelCollector;
use ::vm::Scheduling;
use ::vm::VMScheduling;

pub struct ParallelCollectorGroup<C: ParallelCollector> {
    //name: String,
    contexts: Vec<C>,
    sync: Mutex<ParallelCollectorGroupSync>,
    condvar: Condvar,
}

struct ParallelCollectorGroupSync {
    trigger_count: usize,
    contexts_parked: usize,
    aborted: bool,
    rendezvous_counter: [usize; 2],
    current_rendezvous_counter: usize,
}

impl<C: ParallelCollector> ParallelCollectorGroup<C> {
    pub fn new() -> Self {
        Self {
            contexts: Vec::<C>::new(),
            sync: Mutex::new(ParallelCollectorGroupSync {
                trigger_count: 0,
                contexts_parked: 0,
                aborted: false,
                rendezvous_counter: [0, 0],
                current_rendezvous_counter: 0,
            }),
            condvar: Condvar::new(),
        }
    }

    pub fn active_worker_count(&self) -> usize {
        self.contexts.len()
    }

    pub fn init_group(&mut self, size: usize) {
        {
            let inner = self.sync.get_mut().unwrap();
            inner.trigger_count = 1;
        }
        self.contexts = Vec::<C>::with_capacity(size);
        for i in 0 .. size - 1 {
            self.contexts.push(C::new());
            // XXX: Borrow-checker fighting. I _believe_ this is unavoidable
            //      because we have a circular dependency here, but I'd very
            //      much like to be wrong.
            let self_ptr = self as *const Self;
            self.contexts[i].set_group(self_ptr);
            self.contexts[i].set_worker_ordinal(i);
            VMScheduling::spawn_worker_thread(1, &mut self.contexts[i]); // FIXME
        }
    }

    pub fn trigger_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        inner.trigger_count += 1;
        inner.contexts_parked = 0;
        self.condvar.notify_all();
    }

    pub fn abort_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        if inner.contexts_parked < self.contexts.len() {
            inner.aborted = true;
        }
    }

    // TODO: Can we get away without this lock?
    pub fn is_aborted(&self) -> bool {
        self.sync.lock().unwrap().aborted
    }

    pub fn wait_for_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        while inner.contexts_parked < self.contexts.len() {
            inner = self.condvar.wait(inner).unwrap();
        }
    }

    pub fn park(&self, context: &mut C) {
        // if (VM.VERIFY_ASSERTIONS) VM.assertions._assert(isMember(context));
        let mut inner = self.sync.lock().unwrap();
        context.increment_last_trigger_count();
        if context.get_last_trigger_count() == inner.trigger_count {
            inner.contexts_parked += 1;
            if inner.contexts_parked == inner.trigger_count {
                inner.aborted = false;
            }
            self.condvar.notify_all();
            while context.get_last_trigger_count() == inner.trigger_count {
                inner = self.condvar.wait(inner).unwrap();
            }
        }
    }

    // TODO: is_member?

    pub fn rendezvous(&self) -> usize {
        let mut inner = self.sync.lock().unwrap();
        let i = inner.current_rendezvous_counter;
        let me = inner.rendezvous_counter[i];
        inner.rendezvous_counter[i] += 1;
        if me == self.contexts.len() - 1 {
            inner.current_rendezvous_counter ^= 1;
            inner.rendezvous_counter[inner.current_rendezvous_counter] = 0;
            self.condvar.notify_all();
        } else {
            while inner.rendezvous_counter[i] < self.contexts.len() {
                inner = self.condvar.wait(inner).unwrap();
            }
        }
        me
    }
}