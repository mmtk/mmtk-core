pub mod memory;
pub mod process;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
pub(crate) mod posix_common;
#[cfg(target_os = "linux")]
pub(crate) mod linux;

#[cfg(target_os = "windows")]
pub(crate) mod windows;

pub trait OperatingSystem {
  type OSMemory: memory::Memory;
  type OSProcess: process::Process;
}
