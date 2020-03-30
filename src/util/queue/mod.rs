pub use self::local_queue::LocalQueue;
pub use self::shared_queue::SharedQueue;
use crate::util::constants::LOG_BYTES_IN_PAGE;

mod local_queue;
mod shared_queue;

const TRACE_QUEUE: bool = false;
const LOG_PAGES_PER_BUFFER: usize = 0;
const LOG_BUFFER_SIZE: usize = (LOG_BYTES_IN_PAGE as usize + LOG_PAGES_PER_BUFFER);
const BUFFER_SIZE: usize = 1 << LOG_BUFFER_SIZE;