use ::util::queue::LocalQueue;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::fmt::Debug;

type Block<T> = Vec<T>;

pub struct SharedQueue<T> {
    blocks: Mutex<Vec<Block<T>>>,
    count: AtomicUsize,
    // this stores whether each local queue (by id) is starved (it has done its local work)
    bitmap: Mutex<HashMap<usize, bool>>,
}

impl<T> SharedQueue<T> where T: Debug {
    pub fn new() -> Self {
        SharedQueue {
            blocks: Mutex::new(Vec::new()),
            count: AtomicUsize::new(0),
            bitmap: Mutex::new(HashMap::new()),
        }
    }

    pub fn spin(&self, id: usize) -> Option<Block<T>> {
        // We are locally done
        let mut bitmap = self.bitmap.lock().unwrap();
        bitmap.insert(id, true);
        drop(bitmap);
        'outer: loop {
            let mut blocks = self.blocks.lock().unwrap();
            // Is the global queue empty?
            if !blocks.is_empty() {
                // No, we grab a new block
                let result = blocks.pop().unwrap();
                let mut bitmap = self.bitmap.lock().unwrap();
                // Send the work back, and mark this local queue as not starved.
                bitmap.insert(id, false);
                return Some(result);
            } else {
                // Yes
                let bitmap = self.bitmap.lock().unwrap();
                // Has everyone finished?
                for (_, finished) in bitmap.iter() {
                    if !finished {
                        // No, we busy-wait
                        continue 'outer;
                    }
                }
                // Yes
                break;
            }
        }
        // Everyone is done
        return None;
    }

    pub fn push(&self, b: Block<T>) {
        let mut blocks = self.blocks.lock().unwrap();
        blocks.push(b);
    }

    pub fn spawn_local(&self) -> LocalQueue<T> {
        let mut bitmap = self.bitmap.lock().unwrap();
        let id = self.count.fetch_add(1, Ordering::SeqCst);
        bitmap.insert(id, false);
        LocalQueue::new(id, self)
    }

    pub fn is_empty(&self) -> bool {
        let blocks = self.blocks.lock().unwrap();
        blocks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    extern crate crossbeam;
    use util::queue::SharedQueue;
    use util::test_util::panic_after;

    const SPIN_TIME_OUT: u64 = 200;

    #[test]
    // Calls spin() on a non-empty shared queue. It returns a block immediately.
    fn pull_block() {
        let shared = SharedQueue::<usize>::new();
        let mut local = shared.spawn_local();

        shared.push(vec![42]);
        let res = shared.spin(0);
        assert!(res.is_some());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert_eq!(res.as_ref().unwrap()[0], 42);
    }

    #[test]
    // Calls spin() on an empty shared queue with no other local queue. It returns None immediately.
    fn spin_empty() {
        let shared = SharedQueue::<usize>::new();
        let mut local = shared.spawn_local();
        // This is the only queue. And it is starved. So return None immediately.
        let res = shared.spin(0);
        assert!(res.is_none());
    }

    #[test]
    #[should_panic]
    // Calls spin() on an empty shared queue with other local queue working. It spins and waits.
    fn spin_wait() {
        let shared = SharedQueue::<usize>::new();
        let mut local1 = shared.spawn_local();
        let mut local2 = shared.spawn_local();

        panic_after(SPIN_TIME_OUT, move || {
            // Queue #0 calls spin(). However, Queue #1 is not starved. So we spin-wait here.
            // And this thread will panic if we are blocked for 200ms, which marks the test succeed.
            shared.spin(0);
        });
    }

    #[test]
    // All local queues call spin(). It returns None immediately.
    fn spin_done() {
        let shared = SharedQueue::<usize>::new();

        panic_after(SPIN_TIME_OUT, move || {
            crossbeam::scope(|scope| {
                let mut local1 = shared.spawn_local();
                let mut local2 = shared.spawn_local();
                let worker1 = scope.spawn(|_| {
                    let res = shared.spin(0);
                    assert!(res.is_none());
                });
                let worker2 = scope.spawn(|_| {
                    let res = shared.spin(1);
                    assert!(res.is_none());
                });
                assert!(worker1.join().is_ok());
                assert!(worker2.join().is_ok());
            });
        });
    }

    #[test]
    // One local queue calls spin() and waits until it gets some new work.
    fn spin_return() {
        let shared = SharedQueue::<usize>::new();

        panic_after(SPIN_TIME_OUT, move || {
            crossbeam::scope(|scope| {
                let mut local1 = shared.spawn_local();
                let mut local2 = shared.spawn_local();

                let worker1 = scope.spawn(|_| {
                    let res = shared.spin(0);
                    assert!(res.is_some());
                    assert_eq!(res.as_ref().unwrap().len(), 1);
                    assert_eq!(res.as_ref().unwrap()[0], 42);
                });

                // push some work
                shared.push(vec![42]);

                assert!(worker1.join().is_ok());
            });
        });
    }
}