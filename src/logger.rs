use std::ptr::addr_of;

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::{
    ffi::{c_str, to_c_string},
    host::Host,
    Engine,
};

/// Sends `log` records to metamod's log functions. Metamod prefixes them
/// with the plugin's log tag, marks errors and prints debug and trace lines
/// only while the `developer` cvar is on. Before metamod connects, and off
/// the engine thread, records go to stderr.
///
/// The engine drops logged lines until the first map starts its log, which
/// would hide everything a plugin logs while it attaches at server start.
/// Until then, errors, warnings and info go straight to the console in
/// metamod's format.
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
        let (log, text) = match level {
            Level::Error | Level::Warn | Level::Info if !self.map_started() => {
                let marker = if level == Level::Error { "ERROR: " } else { "" };
                (
                    self.util.pfnLogConsole,
                    format!("[{}] {marker}{text}", self.log_tag()),
                )
            }
            Level::Error => (self.util.pfnLogError, text.to_owned()),
            Level::Warn | Level::Info => (self.util.pfnLogMessage, text.to_owned()),
            Level::Debug | Level::Trace => (self.util.pfnLogDeveloper, text.to_owned()),
        };
        let text = to_c_string(&text);
        match log {
            // SAFETY: the text goes in as a `%s` argument, so a `%` in it is
            // printed instead of read as a conversion, and the plugin id is
            // this plugin's static info.
            Some(log) => unsafe { log(self.plugin_id, c"%s".as_ptr(), text.as_ptr()) },
            None => eprintln!("{}", text.to_string_lossy()),
        }
    }

    fn map_started(&self) -> bool {
        // SAFETY: `globals` points to the engine globals while loaded.
        unsafe { addr_of!((*self.globals.as_ptr()).mapname).read() != 0 }
    }

    fn log_tag(&self) -> String {
        // SAFETY: the plugin id points to the plugin's static info.
        unsafe { c_str(addr_of!((*self.plugin_id).logtag).read()) }
            .map(|tag| tag.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}
