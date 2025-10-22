use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};
use crate::util::VMMutatorThread;

#[derive(Clone)]
pub struct ThreadPark {
    inner: Arc<Inner>,
}

struct Inner {
    lock: Mutex<State>,
    cvar: Condvar,
}

#[derive(Default)]
struct State {
    /// All registered parked and whether they are currently parked.
    parked: HashMap<VMMutatorThread, bool>,
}

impl ThreadPark {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                lock: Mutex::new(State::default()),
                cvar: Condvar::new(),
            }),
        }
    }

    /// Register the current thread for coordination.
    pub fn register(&self, tid: VMMutatorThread) {
        info!("Register {:?} to ThreadPark", tid);
        let mut state = self.inner.lock.lock().unwrap();
        state.parked.insert(tid, false);
    }

    pub fn unregister(&self, tid: VMMutatorThread) {
        let mut state = self.inner.lock.lock().unwrap();
        state.parked.remove(&tid);
    }

    pub fn is_thread(&self, tid: VMMutatorThread) -> bool {
        let state = self.inner.lock.lock().unwrap();
        state.parked.contains_key(&tid)
    }

    pub fn number_of_threads(&self) -> usize {
        let state = self.inner.lock.lock().unwrap();
        state.parked.len()
    }

    pub fn all_threads(&self) -> Vec<VMMutatorThread> {
        let state = self.inner.lock.lock().unwrap();
        state.parked.keys().cloned().collect()
    }

    /// Park the current thread (set its state = parked and wait for unpark_all()).
    pub fn park(&self, tid: VMMutatorThread) {
        let mut state = self.inner.lock.lock().unwrap();

        // Mark this thread as parked
        if let Some(entry) = state.parked.get_mut(&tid) {
            *entry = true;
        } else {
            panic!("Thread {:?} not registered before park()", tid);
        }

        // Notify any waiter that one more thread has parked
        self.inner.cvar.notify_all();

        // Wait until unpark_all() is called
        state = self.inner.cvar.wait(state).unwrap();

        // Mark this thread as unparked again
        if let Some(entry) = state.parked.get_mut(&tid) {
            *entry = false;
        }
    }

    /// Unpark all registered threads (wake everyone up).
    pub fn unpark_all(&self) {
        let mut state = self.inner.lock.lock().unwrap();
        for v in state.parked.values_mut() {
            *v = false;
        }
        self.inner.cvar.notify_all();
    }

    /// Block until all registered threads are parked.
    pub fn wait_all_parked(&self) {
        let mut state = self.inner.lock.lock().unwrap();
        loop {
            let all_parked = !state.parked.is_empty()
                && state.parked.values().all(|&v| v);
            if all_parked {
                break;
            }
            state = self.inner.cvar.wait(state).unwrap();
        }
    }
}
