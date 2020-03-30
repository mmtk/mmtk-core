use super::{BUFFER_SIZE, TRACE_QUEUE};
use crate::util::queue::SharedQueue;
use std::fmt::Debug;
use std::mem;

pub struct LocalQueue<'a, T: 'a> {
    queue: &'a SharedQueue<T>,
    buffer: Vec<T>,
    id: usize,
}

impl<'a, T> LocalQueue<'a, T>
where
    T: Debug,
{
    pub fn new(id: usize, queue: &'a SharedQueue<T>) -> Self {
        LocalQueue {
            queue,
            buffer: Vec::with_capacity(BUFFER_SIZE),
            id,
        }
    }

    pub fn enqueue(&mut self, v: T) {
        if TRACE_QUEUE {
            println!("len {:?}", v);
        }
        if self.buffer.len() >= BUFFER_SIZE {
            let mut b = Vec::with_capacity(BUFFER_SIZE);
            mem::swap(&mut b, &mut self.buffer);
            self.queue.push(b);
            self.enqueue(v);
        } else {
            self.buffer.push(v);
        }
    }

    pub fn dequeue(&mut self) -> Option<T> {
        if !self.buffer.is_empty() {
            let result = self.buffer.pop();
            if TRACE_QUEUE {
                println!("lde {:?}", result);
            }
            result
        } else {
            match self.queue.spin(self.id) {
                Some(b) => {
                    self.buffer = b;
                    self.dequeue()
                }
                None => None,
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use crate::util::queue::{SharedQueue, BUFFER_SIZE};

    #[test]
    fn new_local_queues() {
        let shared = SharedQueue::<usize>::new();
        let local1 = shared.spawn_local();
        assert_eq!(local1.id, 0);
        let local2 = shared.spawn_local();
        assert_eq!(local2.id, 1);
    }

    #[test]
    fn enqueue_dequeue_buffer() {
        let shared = SharedQueue::<usize>::new();
        let mut local = shared.spawn_local();

        // Fill local buffer
        for i in 0..BUFFER_SIZE {
            local.enqueue(i);
            assert_eq!(local.buffer.len(), i + 1)
        }
        assert!(shared.is_empty());

        // Pop local buffer
        for i in (0..BUFFER_SIZE).rev() {
            let res = local.dequeue().unwrap();
            assert_eq!(res, i);
        }
    }

    #[test]
    fn enqueue_dequeue_shared() {
        let shared = SharedQueue::<usize>::new();
        let mut local = shared.spawn_local();

        // Fill local buffer
        for i in 0..BUFFER_SIZE {
            local.enqueue(i);
        }
        assert!(shared.is_empty());

        // Make local queue flush buffer to shared queue
        local.enqueue(42);
        assert_eq!(local.buffer.len(), 1);
        assert_eq!(local.buffer[0], 42);
        assert!(!shared.is_empty());

        // Dequeue from local buffer
        let res = local.dequeue().unwrap();
        assert_eq!(res, 42);
        // Dequeue from shared queue
        for i in (0..BUFFER_SIZE).rev() {
            let res = local.dequeue().unwrap();
            assert!(shared.is_empty());
            assert_eq!(res, i);
        }
    }
}
