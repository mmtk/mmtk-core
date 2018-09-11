use std::thread;
use std::sync::mpsc;
use crossbeam_deque::{self as deque, Steal, Stealer};

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