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

cfg_if::cfg_if! {
    if #[cfg(all(not(debug_assertions), not(feature = "hot_log")))] {
        // If it is release build and the feature "hot_log" is not enabled,
        // then we define verbose logs as no-op in release build.

        /// The `log::debug!` macro is disabled in release build.
        /// Use the "hot_log" feature to enable.
        macro_rules! debug {
            ($($arg:tt)+) => {}
        }

        /// The `log::trace!` macro is disabled in release build.
        /// Use the "hot_log" feature to enable.
        macro_rules! trace {
            ($($arg:tt)+) => {}
        }

        // By default, a macro has no path-based scope.
        // The following allows other modules to access the macros with `crate::util::log::debug`
        // and `crate::util::log::trace`.
        pub(crate) use debug;
        pub(crate) use trace;

    } else {
        // Otherwise simply import the macros from the `log` crate.
        pub(crate) use the_log_crate::{debug, trace};
    }
}
