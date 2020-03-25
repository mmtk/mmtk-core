use log::{self, Log, Record, Metadata, SetLoggerError, LevelFilter};
use std::env;
use std::thread;

/// Adapted from SimpleLogger in crate `log`
struct MMTkLogger;

impl Log for MMTkLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        // Cap it at compilation time
        // If built with debug, can be tweaked using "RUST_LOG" env var.
        true
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!("{:?}[{}:{}:{}] {}",
                     thread::current().id(),
                     record.level(),
                     record.file().unwrap(),
                     record.line().unwrap(),
                     record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: MMTkLogger = MMTkLogger;

pub fn init() -> Result<(), SetLoggerError> {
    match env::var("RUST_LOG") {
        Ok(log_level) => match log_level.as_ref() {
            "OFF" => log::set_max_level(LevelFilter::Off),
            "ERROR" => log::set_max_level(LevelFilter::Error),
            "WARN" => log::set_max_level(LevelFilter::Warn),
            "INFO" => log::set_max_level(LevelFilter::Info),
            "DEBUG" => log::set_max_level(LevelFilter::Debug),
            "TRACE" => log::set_max_level(LevelFilter::Trace),
            _ => log::set_max_level(LevelFilter::Info),
        }
        Err(_) => log::set_max_level(LevelFilter::Info)
    }
    log::set_logger(&LOGGER)
}