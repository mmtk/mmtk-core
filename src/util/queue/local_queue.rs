use ::util::queue::SharedQueue;
use std::fmt::Debug;
use std::mem;
use super::{BUFFER_SIZE, TRACE_QUEUE};

pub struct LocalQueue<'a, T: 'a> {
    queue: &'a SharedQueue<T>,
    buffer: Vec<T>,
    id: usize,
}

impl<'a, T> LocalQueue<'a, T> where T: Debug {
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
            return result;
        } else {
            match self.queue.spin(self.id) {
                Some(b) => {
                    self.buffer = b;
                    return self.dequeue();
                }
                None => None
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