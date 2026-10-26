#![feature(trivial_bounds)]
#![deny(unsafe_op_in_unsafe_fn)]

mod command;
mod cvar;
mod engine;
mod entity;
#[doc(hidden)] pub mod export;
mod ffi;
mod hooks;
mod host;
mod logger;
mod main_thread;
mod message;
mod meta;
mod plugin;

pub use command::CommandArgs;
pub use cvar::{Cvar, CvarError, CvarFlags, StaticCvar};
pub use engine::{AlertKind, Engine, PrintKind};
pub use entity::Edict;
pub use main_thread::MainThreadCell;
pub use message::{MessageDestination, MessageError, MessageType, MessageWriter};
pub use meta::{MetaResult, Returned};
pub use metamod_p_macros::plugin;
pub use plugin::{LoadTime, Overrides, Plugin, PluginInfo, RejectReason, UnloadReason};

/// Raw bindgen output for the selected metamod headers (`fallguys` feature:
/// metamod-svencoop, default: Metamod-P). Everything here is `unsafe` to use.
#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]
pub mod sys {
    use const_default::ConstDefault;

    include!(concat!(env!("OUT_DIR"), "/wrapper.rs"));
}
