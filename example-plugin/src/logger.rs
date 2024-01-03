use std::ffi::CString;

use log::{Level, Metadata, Record};

use crate::{gpMetaUtilFuncs, Plugin_info};
pub struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let msg = if record.metadata().level() == Level::Error {
                format!("{}", record.args())
            } else {
                format!("{}: {}", record.level(), record.args())
            };
            let msg: CString = CString::new(msg).unwrap();
            if let Some(utils) = unsafe { gpMetaUtilFuncs.as_ref() } {
                match record.metadata().level() {
                    Level::Error => {
                        if let Some(pfnLogError) = utils.pfnLogError {
                            unsafe { pfnLogError(&mut Plugin_info, msg.as_ptr()) }
                        }
                    }
                    Level::Warn | Level::Info => {
                        if let Some(pfnLogMessage) = utils.pfnLogMessage {
                            unsafe { pfnLogMessage(&mut Plugin_info, msg.as_ptr()) }
                        }
                    }
                    Level::Debug | Level::Trace => {
                        if let Some(pfnLogDeveloper) = utils.pfnLogDeveloper {
                            unsafe { pfnLogDeveloper(&mut Plugin_info, msg.as_ptr()) }
                        }
                    }
                }
            }
        }
    }

    fn flush(&self) {}
}
