use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::{ffi::to_c_string, host::Host, Engine};

/// Sends `log` records to metamod's log functions. Metamod prefixes them
/// with the plugin's log tag, marks errors and prints debug and trace lines
/// only while the `developer` cvar is on. Before metamod connects, and off
/// the engine thread, records go to stderr.
struct MetamodLogger;

impl Log for MetamodLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let text = match record.level() {
            Level::Warn => format!("WARNING: {}", record.args()),
            _ => record.args().to_string(),
        };
        match Host::engine() {
            Some(engine) => engine.log(record.level(), &text),
            None => eprintln!("{text}"),
        }
    }

    fn flush(&self) {}
}

/// Installs the logger with the `Info` level, unless a logger is installed
/// already. Plugins can change the level with `log::set_max_level`.
pub(crate) fn install() {
    if log::set_logger(&MetamodLogger).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}

impl Engine {
    fn log(&self, level: Level, text: &str) {
        let text = to_c_string(text);
        let log = match level {
            Level::Error => self.util.pfnLogError,
            Level::Warn | Level::Info => self.util.pfnLogMessage,
            Level::Debug | Level::Trace => self.util.pfnLogDeveloper,
        };
        match log {
            // SAFETY: the text goes in as a `%s` argument, so a `%` in it is
            // printed instead of read as a conversion, and the plugin id is
            // this plugin's static info.
            Some(log) => unsafe { log(self.plugin_id, c"%s".as_ptr(), text.as_ptr()) },
            None => eprintln!("{}", text.to_string_lossy()),
        }
    }
}
