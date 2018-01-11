use std::vec::Vec;
use std::sync::{Mutex, Condvar};

use super::ParallelCollector;

pub struct ParallelCollectorGroup<C: ParallelCollector> {
    name: String,
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
    fn active_worker_count(&self) -> usize {
        self.contexts.len()
    }

    fn init_group(&mut self, size: usize) {
        let inner = self.sync.get_mut().unwrap();
        inner.trigger_count = 1;
        self.contexts = Vec::<C>::with_capacity(size);
        for i in 0 .. size - 1 {
            //self.contexts.push();
        }
        unimplemented!();
    }

    fn trigger_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        inner.trigger_count += 1;
        inner.contexts_parked = 0;
        self.condvar.notify_all();
    }

    fn abort_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        if inner.contexts_parked < self.contexts.len() {
            inner.aborted = true;
        }
    }

    // TODO: Can we get away without this lock?
    fn is_aborted(&self) -> bool {
        self.sync.lock().unwrap().aborted
    }

    fn wait_for_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        while inner.contexts_parked < self.contexts.len() {
            inner = self.condvar.wait(inner).unwrap();
        }
    }

    fn park(&self, context: C) {
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

    fn rendezvous(&self) -> usize {
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