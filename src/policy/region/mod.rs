mod region;
mod regionspace;
pub mod cardtable;
mod card;
mod remset;
mod marktable;

pub use self::region::*;
pub use self::regionspace::*;
pub use self::card::*;
pub use self::remset::*;
pub use self::marktable::*;
pub use self::cardtable::*;

const DEBUG: bool = false;
