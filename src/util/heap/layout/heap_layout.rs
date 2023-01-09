pub use super::map::Map;
pub use crate::util::heap::layout::map32::Map32;
pub use crate::util::heap::layout::map64::Map64;

pub use super::Mmapper;
pub use crate::util::heap::layout::ByteMapMmapper as Mmapper32;
pub use crate::util::heap::layout::FragmentedMapper as Mmapper64;
