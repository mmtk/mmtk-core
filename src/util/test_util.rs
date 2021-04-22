use std::panic;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

// https://github.com/rust-lang/rfcs/issues/2798#issuecomment-552949300
pub fn panic_after<T, F>(millis: u64, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T,
    F: Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let val = f();
        done_tx.send(()).expect("Unable to send completion signal");
        val
    });

    match done_rx.recv_timeout(Duration::from_millis(millis)) {
        Ok(_) => handle.join().expect("Thread panicked"),
        Err(e) => panic!("Thread took too long: {}", e),
    }
}

lazy_static! {
    static ref SERIAL_TEST_LOCK: Mutex<()> = Mutex::default();
}

// force some tests to be executed serially
pub fn serial_test<F>(f: F)
where
    F: FnOnce(),
{
    let _lock = SERIAL_TEST_LOCK.lock();
    f();
}

// Always execute a cleanup closure no matter the test panics or not.
pub fn with_cleanup<T, C>(test: T, cleanup: C)
where
    T: FnOnce() + panic::UnwindSafe,
    C: FnOnce(),
{
    let res = panic::catch_unwind(test);
    cleanup();
    if let Err(e) = res {
        panic::resume_unwind(e);
    }
}
