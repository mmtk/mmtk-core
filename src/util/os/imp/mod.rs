#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(target_os = "windows")]
pub use windows::WindowsMemoryImpl as OSMemory;
#[cfg(target_os = "windows")]
pub use windows::WindowsProcessImpl as OSProcess;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
pub(crate) mod unix_like;

#[cfg(target_os = "linux")]
pub use unix_like::linux_like::linux::LinuxMemoryImpl as OSMemory;
#[cfg(target_os = "linux")]
pub use unix_like::linux_like::linux::LinuxProcessImpl as OSProcess;

#[cfg(target_os = "android")]
pub use unix_like::linux_like::android::AndroidMemoryImpl as OSMemory;
#[cfg(target_os = "android")]
pub use unix_like::linux_like::android::AndroidProcessImpl as OSProcess;

#[cfg(target_os = "macos")]
pub use unix_like::macos::MacOSMemoryImpl as OSMemory;
#[cfg(target_os = "macos")]
pub use unix_like::macos::MacOSProcessImpl as OSProcess;
