//! This module provides a built-in logger implementation.
//!
//! The built-in logger implementation uses the `env_logger` crate.  It is enabled by the Cargo
//! feature "builtin_env_logger" which is enabled by default.  When enabled, it will be initialized
//! in [`crate::memory_manager::mmtk_init`] and will show logs of levels INFO or lower (the lower,
//! the more important).
//!
//! This provides convenient out-of-the-box experience for binding developers so that they can see
//! logs when using MMTk without configuration, and can easily configure log levels from environment
//! variables.  Some bindings may wish to choose a different implementation, or implement their own
//! logging implementations to integrate with the existing logging frameworks of their VMs.  In such
//! cases, the binding can disable the Cargo feature "builtin_env_logger" and register their own
//! implementations with the `log` crate.

/// Attempt to init a env_logger for MMTk.
/// Does nothing if the "builtin_env_logger" feature is disabled.
pub(crate) fn try_init() {
    cfg_if::cfg_if! {
        if #[cfg(feature = "builtin_env_logger")] {
            let result = env_logger::try_init_from_env(
                // By default, show info level logging.
                env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
            );

            match result {
                Ok(()) => {
                    debug!("MMTk initialized the logger.");
                }
                Err(e) => {
                    // Currently `log::SetLoggerError` can only be raised for one reason: the logger has already been initialized.
                    debug!("MMTk failed to initialize the built-in env_logger: {e}");
                }
            }
        } else {
            debug!("MMTk didn't initialize the built-in env_logger.  The Cargo feature \"builtin_env_logger\" is not enabled.");
        }
    }
}
