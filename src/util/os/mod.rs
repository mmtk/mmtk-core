//! Operating System abstractions for MMTk.

// Note:
// 1. For functions that return `Result`, an error value should only be used for exceptional cases. If a function returns
// a placeholder value, that should not be considered as 'exceptional cases', and should return Ok.
// 2. Some functions or arguments (e.g. [`crate::util::os::memory::MmapStrategy`]) allow fallback behaviors for platforms where certain features
// are not supported, or unimplemented.

mod memory;
pub use memory::*;
mod process;
pub use process::*;

mod imp;
pub use imp::OS;

trait OperatingSystem: OSMemory + OSProcess {}
