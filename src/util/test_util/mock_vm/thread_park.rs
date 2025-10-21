use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone)]
pub struct ThreadPark {
    inner: Arc<Inner>,
}

struct Inner {
    lock: Mutex<()>,
    cvar: Condvar,
}

impl ThreadPark {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                lock: Mutex::new(()),
                cvar: Condvar::new(),
            }),
        }
    }

    /// Park the current thread until `unpark_all()` is called.
    pub fn park(&self) {
        let guard = self.inner.lock.lock().unwrap();
        // Wait until notified; condvar wait automatically unlocks/relocks the mutex.
        // If spurious wakeups happen, the caller should re-check its condition.
        let _unused = self.inner.cvar.wait(guard).unwrap();
    }

    /// Wake up all threads currently parked on `park()`.
    pub fn unpark_all(&self) {
        self.inner.cvar.notify_all();
    }
}
