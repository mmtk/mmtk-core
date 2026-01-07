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

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
pub(crate) mod posix_common;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(target_os = "windows")]
pub use windows::WindowsMemoryImpl as OSMemory;
#[cfg(target_os = "windows")]
pub use windows::WindowsProcessImpl as OSProcess;

#[cfg(target_os = "linux")]
pub use linux::LinuxMemoryImpl as OSMemory;
#[cfg(target_os = "linux")]
pub use linux::LinuxProcessImpl as OSProcess;

#[cfg(target_os = "macos")]
pub use macos::MacOSMemoryImpl as OSMemory;
#[cfg(target_os = "macos")]
pub use macos::MacOSProcessImpl as OSProcess;
