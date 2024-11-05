use log::SetLoggerError;

/// Attempt to init a env_logger for MMTk.
/// Does nothing if the "builtin_env_logger" feature is disabled.
pub fn try_init() -> Result<(), SetLoggerError> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "builtin_env_logger")] {
            env_logger::try_init_from_env(
                // By default, use info level logging.
                env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
            )
        } else {
            Ok(())
        }
    }
}
