//! This module provides utilities for logging
//!
//! It provide wrappers for the logging macros in the `log` crate.  Those macros are used the same
//! way as those in the `log` crate, except that log level `DEBUG` and `TRACE` are disabled by
//! default in release build.  They will not be compiled into the resulting binary.  But they can be
//! enabled by the "hot_log" Cargo feature so that they will be displayed in release build, too.
//! This module is named `log` so that programmers can comfortably write `log::info!` as if the
//! macro were from the `log` crate.

// This is just the `log` crate.  We renamed it in `Cargo.toml` so that we don't accidentally import
// macros such as `log::info!` from the IDE.
use the_log_crate;

pub(crate) use the_log_crate::{error, info, warn};

/// Whether logs of DEBUG and TRACE levels are enabled.
/// In debug build, they are always enabled.
/// In release build, they are not enabled unless the "hot_log" Cargo feature is enabled.
pub(crate) const HOT_LOG_ENABLED: bool = cfg!(any(not(debug_assertions), feature = "hot_log"));

/// A wrapper of the `debug!` macro in the `log` crate.
/// Does nothing if [`HOT_LOG_ENABLED`] is false.
macro_rules! debug {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::util::log::HOT_LOG_ENABLED {
            the_log_crate::debug!(target: $target, $($arg)+)
        }
    };
    ($($arg:tt)+) => {
        if $crate::util::log::HOT_LOG_ENABLED {
            the_log_crate::debug!($($arg)+)
        }
    }
}

/// A wrapper of the `trace!` macro in the `log` crate.
/// Does nothing if [`HOT_LOG_ENABLED`] is false.
macro_rules! trace {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::util::log::HOT_LOG_ENABLED {
            the_log_crate::trace!(target: $target, $($arg)+)
        }
    };
    ($($arg:tt)+) => {
        if $crate::util::log::HOT_LOG_ENABLED {
            the_log_crate::trace!($($arg)+)
        }
    }
}

// By default, a macro has no path-based scope.
// The following allows other modules to access the macros with `crate::util::log::debug`
// and `crate::util::log::trace`.
pub(crate) use debug;
pub(crate) use trace;
