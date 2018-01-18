pub mod alloc;
pub mod heap;
pub mod address;
pub mod forwarding_word;
pub mod header_byte;

pub use self::address::Address;
pub use self::address::ObjectReference;