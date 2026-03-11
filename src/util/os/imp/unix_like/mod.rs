pub mod unix_common;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod linux_like;
