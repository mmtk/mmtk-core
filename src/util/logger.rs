//! This module sets up `env_logger` as the default logger.

use the_log_crate::{self, SetLoggerError};

/// Attempt to init a env_logger for MMTk.
pub fn try_init() -> Result<(), SetLoggerError> {
    env_logger::try_init_from_env(
        // By default, use info level logging.
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    )
}
