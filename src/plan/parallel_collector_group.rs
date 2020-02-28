use std::vec::Vec;
use std::sync::{Mutex, Condvar};
use ::util::OpaquePointer;
use std::sync::atomic::{AtomicBool, Ordering};

use super::ParallelCollector;
use ::vm::Collection;
use plan::selected_plan::SelectedPlan;

use libc::c_void;
use plan::phase::PhaseManager;
use mmtk::MMTK;
use vm::VMBinding;
use std::marker::PhantomData;

pub struct ParallelCollectorGroup<VM: VMBinding, C: ParallelCollector<VM>> {
    //name: String,
    contexts: Vec<C>,
    sync: Mutex<ParallelCollectorGroupSync>,
    condvar: Condvar,

    aborted: AtomicBool,

    p: PhantomData<VM>,
}

struct ParallelCollectorGroupSync {
    trigger_count: usize,
    contexts_parked: usize,
    rendezvous_counter: [usize; 2],
    current_rendezvous_counter: usize,
}

impl<VM: VMBinding, C: ParallelCollector<VM>> ParallelCollectorGroup<VM, C> {
    pub fn new() -> Self {
        Self {
            contexts: Vec::<C>::new(),
            sync: Mutex::new(ParallelCollectorGroupSync {
                trigger_count: 0,
                contexts_parked: 0,
                rendezvous_counter: [0, 0],
                current_rendezvous_counter: 0,
            }),
            condvar: Condvar::new(),

            aborted: AtomicBool::new(false),
            p: PhantomData,
        }
    }

    pub fn active_worker_count(&self) -> usize {
        self.contexts.len()
    }

    pub fn init_group(&mut self, mmtk: &'static MMTK<VM>, tls: OpaquePointer) {
        {
            let inner = self.sync.get_mut().unwrap();
            inner.trigger_count = 1;
        }
        let size = mmtk.options.threads;
        self.contexts = Vec::<C>::with_capacity(size);
        for i in 0..size {
            self.contexts.push(C::new(mmtk));
            // XXX: Borrow-checker fighting. I _believe_ this is unavoidable
            //      because we have a circular dependency here, but I'd very
            //      much like to be wrong.
            let self_ptr = self as *const Self;
            self.contexts[i].set_group(self_ptr);
            self.contexts[i].set_worker_ordinal(i);
            unsafe {
                VM::VMCollection::spawn_worker_thread(tls, Some(&mut self.contexts[i]));
            }
        }
    }

    pub fn trigger_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        inner.trigger_count += 1;
        inner.contexts_parked = 0;
        self.condvar.notify_all();
    }

    pub fn abort_cycle(&self) {
        let inner = self.sync.lock().unwrap();
        if inner.contexts_parked < self.contexts.len() {
            self.aborted.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Relaxed)
    }

    pub fn wait_for_cycle(&self) {
        let mut inner = self.sync.lock().unwrap();
        while inner.contexts_parked < self.contexts.len() {
            debug!("Inside wait_for_cycle loop, with {}/{} contexts parked", inner.contexts_parked, self.contexts.len());
            inner = self.condvar.wait(inner).unwrap();
        }
    }

    pub fn park(&self, context: &mut C) {
        debug!("Parking context");
        // FIXME if (VM.VERIFY_ASSERTIONS) VM.assertions._assert(isMember(context));
        let mut inner = self.sync.lock().unwrap();
        context.increment_last_trigger_count();
        if context.get_last_trigger_count() == inner.trigger_count {
            inner.contexts_parked += 1;
            if inner.contexts_parked == inner.trigger_count {
                self.aborted.store(false, Ordering::Relaxed);
            }
            self.condvar.notify_all();
            while context.get_last_trigger_count() == inner.trigger_count {
                inner = self.condvar.wait(inner).unwrap();
            }
        }
    }

    pub fn is_member(&self, context: &C) -> bool {
        if let Some(i) = self.contexts.iter().position(|ref c| c.get_tls() == context.get_tls()) {
            true
        } else {
            false
        }
    }

    pub fn rendezvous(&self) -> usize {
        let mut inner = self.sync.lock().unwrap();
        let i = inner.current_rendezvous_counter;
        let me = inner.rendezvous_counter[i];
        inner.rendezvous_counter[i] += 1;
        if me == self.contexts.len() - 1 {
            inner.current_rendezvous_counter ^= 1;
            {
                let index = inner.current_rendezvous_counter;
                inner.rendezvous_counter[index] = 0;
            }
            self.condvar.notify_all();
        } else {
            while inner.rendezvous_counter[i] < self.contexts.len() {
                inner = self.condvar.wait(inner).unwrap();
            }
        }
        me
    }
}
