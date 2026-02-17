#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
pub(crate) mod unix_like;

#[cfg(target_os = "linux")]
pub use unix_like::linux_like::linux::Linux as OS;

#[cfg(target_os = "android")]
pub use unix_like::linux_like::android::Android as OS;

#[cfg(target_os = "macos")]
pub use unix_like::macos::MacOS as OS;
