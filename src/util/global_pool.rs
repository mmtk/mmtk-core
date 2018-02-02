use std::thread;
use std::sync::mpsc;
use crossbeam_deque::{Deque, Steal, Stealer};

pub fn new<T: 'static>(name: &'static str) -> (Stealer<T>, mpsc::Sender<T>) where T: Send {
    let d = Deque::new();
    let (tx, rx) = mpsc::channel();
    let stealer = d.stealer().clone();
    thread::spawn(move || {
        loop {
            let recv = rx.recv();
            match recv {
                Ok(work) => {
                    d.push(work);
                    trace!("GlobalPool({}) len: {}", name, d.len());
                },
                Err(_) => break
            }
        }
    });
    (stealer, tx)
}