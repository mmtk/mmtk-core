use crate::util::VMThread;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone)]
pub struct ThreadPark {
    name: &'static str,
    inner: Arc<Inner>,
}

struct Inner {
    lock: Mutex<State>,
    cvar: Condvar,
}

#[derive(Default)]
struct State {
    /// All registered parked and whether they are currently parked.
    parked: HashMap<VMThread, bool>,
}

impl ThreadPark {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            inner: Arc::new(Inner {
                lock: Mutex::new(State::default()),
                cvar: Condvar::new(),
            }),
        }
    }

    /// Register the current thread for coordination.
    pub fn register(&self, tid: VMThread) {
        debug!("Register {:?} to {}", tid, self.name);
        let mut state = self.inner.lock.lock().unwrap();
        state.parked.insert(tid, false);
    }

    pub fn unregister(&self, tid: VMThread) {
        let mut state = self.inner.lock.lock().unwrap();
        state.parked.remove(&tid);
    }

    pub fn contains(&self, tid: VMThread) -> bool {
        let state = self.inner.lock.lock().unwrap();
        state.parked.contains_key(&tid)
    }

    pub fn number_of_threads(&self) -> usize {
        let state = self.inner.lock.lock().unwrap();
        state.parked.len()
    }

    pub fn all_threads(&self) -> Vec<VMThread> {
        let state = self.inner.lock.lock().unwrap();
        state.parked.keys().cloned().collect()
    }

    /// Park the current thread (set its state = parked and wait for unpark_all()).
    pub fn park(&self, tid: VMThread) {
        let mut state = self.inner.lock.lock().unwrap();

        // Mark this thread as parked
        if let Some(entry) = state.parked.get_mut(&tid) {
            *entry = true;
        } else {
            panic!(
                "Thread {:?} not registered to {} before park() f",
                tid, self.name
            );
        }

        // Notify any waiter that one more thread has parked
        self.inner.cvar.notify_all();

        // Wait until unpark_all() clears the parked flag of this thread. We need to check the flag,
        // as the thread may wake up spuriously, or be woken up by the notification above when
        // another thread parks. The thread cannot be unregistered while it is parked.
        let _state = self
            .inner
            .cvar
            .wait_while(state, |state| {
                *state.parked.get(&tid).unwrap_or_else(|| {
                    panic!(
                        "Thread {:?} unregistered from {} during park()",
                        tid, self.name
                    )
                })
            })
            .unwrap();
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
        let state = self.inner.lock.lock().unwrap();
        let _state = self
            .inner
            .cvar
            .wait_while(state, |state| {
                state.parked.is_empty() || !state.parked.values().all(|&v| v)
            })
            .unwrap();
    }
}
