use std::time::Duration;
use std::sync::mpsc;
use std::thread;

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
        Err(_) => panic!("Thread took too long"),
    }
}