use log::SetLoggerError;

/// Failure of setting logger.
pub(crate) enum LoggerError {
    /// The user didn't enable the "builtin_env_logger" feature.
    NoBuiltinLogger,
    /// Error happened while setting the logger.
    SetLoggerError(SetLoggerError),
}

/// Attempt to init a env_logger for MMTk.
/// Does nothing if the "builtin_env_logger" feature is disabled.
pub fn try_init() -> Result<(), LoggerError> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "builtin_env_logger")] {
            env_logger::try_init_from_env(
                // By default, use info level logging.
                env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
            ).map_err(LoggerError::SetLoggerError)
        } else {
            Err(LoggerError::NoBuiltinLogger)
        }
    }
}
