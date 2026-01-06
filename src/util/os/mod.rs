mod memory;
pub use memory::*;
mod process;
pub use process::*;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
pub(crate) mod posix_common;
#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
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

// pub trait OperatingSystem {
//   type OSMemory: memory::Memory;
//   type OSProcess: process::Process;
// }
