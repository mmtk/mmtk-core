use std::thread;
use std::sync::mpsc;
use crossbeam_deque::{self as deque, Steal, Stealer};

// FIXME we have a race condition here that after GC workers check that the global queue is empty,
// the coordinator thread might still add work to the queue
// Thus, we might have residual work after all GC threads finish the cycle
pub fn new<T: 'static>(name: &'static str) -> (Stealer<T>, mpsc::Sender<T>) where T: Send {
    let (w, s) = deque::fifo();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let recv = rx.recv();
            match recv {
                Ok(work) => {
                    w.push(work);
                },
                Err(_) => break
            }
        }
    });
    (s, tx)
}