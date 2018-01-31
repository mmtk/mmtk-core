pub mod alloc;
pub mod heap;
pub mod address;
pub mod forwarding_word;
pub mod header_byte;
pub mod logger;
pub mod constants;
#[macro_use]
pub mod macros;
mod synchronized_counter;

pub use self::address::Address;
pub use self::address::ObjectReference;
pub use self::synchronized_counter::SynchronizedCounter;