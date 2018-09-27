use ::util::constants::LOG_BYTES_IN_PAGE;
use ::util::shared_queue::SharedQueue;
use std::mem;

const LOG_PAGES_PER_BUFFER: usize = 0;
const PAGES_PER_BUFFER: usize = 1 << LOG_PAGES_PER_BUFFER;
const LOG_BUFFER_SIZE: usize = (LOG_BYTES_IN_PAGE as usize + LOG_PAGES_PER_BUFFER);
const BUFFER_SIZE: usize = 1 << LOG_BUFFER_SIZE;

pub struct LocalQueue<'a, T> {
    queue: &'a SharedQueue<T>,
    buffer: Vec<T>,
    id: usize,
}

impl<'a, T> LocalQueue<'a, T> {
    pub fn new(id: usize, queue: &'a SharedQueue<T>) -> Self {
        LocalQueue {
            queue,
            buffer: Vec::with_capacity(BUFFER_SIZE),
            id,
        }
    }

    pub fn enqueue(&mut self, v: T) {
        if self.buffer.len() >= BUFFER_SIZE {
            let mut b = Vec::with_capacity(BUFFER_SIZE);
            mem::swap(&mut b, &mut self.buffer);
            self.queue.push(b);
        } else {
            self.buffer.push(v);
        }
    }

    pub fn dequeue(&mut self) -> Option<T> {
        if !self.buffer.is_empty() {
            return self.buffer.pop();
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
}