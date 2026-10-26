use std::{
    error::Error,
    ffi::{c_int, CStr},
    fmt, ptr,
};

use crate::{
    engine::host_call,
    sys::{self, edict_t},
    Edict, Engine,
};

/// Who receives a message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MessageDestination {
    /// Every player, unreliable (`MSG_BROADCAST`).
    Broadcast,
    /// Every player, reliable (`MSG_ALL`).
    All,
    /// One player, reliable (`MSG_ONE`).
    One(Edict),
    /// One player, unreliable (`MSG_ONE_UNRELIABLE`).
    OneUnreliable(Edict),
    /// Players who can see the origin, unreliable (`MSG_PVS`).
    Visible([f32; 3]),
    /// Players who can see the origin, reliable (`MSG_PVS_R`).
    VisibleReliable([f32; 3]),
    /// Players who can hear the origin, unreliable (`MSG_PAS`).
    Audible([f32; 3]),
    /// Players who can hear the origin, reliable (`MSG_PAS_R`).
    AudibleReliable([f32; 3]),
    /// The signon data that every new client receives (`MSG_INIT`).
    Init,
    /// Spectators and HLTV proxies (`MSG_SPEC`).
    Spectators,
}

impl MessageDestination {
    /// The `MSG_*` code, the origin and the receiving edict for MessageBegin.
    fn raw(&self) -> (c_int, Option<&[f32; 3]>, *mut edict_t) {
        let code = |destination: u32| destination as c_int;
        match self {
            Self::Broadcast => (code(sys::MSG_BROADCAST), None, ptr::null_mut()),
            Self::All => (code(sys::MSG_ALL), None, ptr::null_mut()),
            Self::One(player) => (code(sys::MSG_ONE), None, player.as_ptr()),
            Self::OneUnreliable(player) => (code(sys::MSG_ONE_UNRELIABLE), None, player.as_ptr()),
            Self::Visible(origin) => (code(sys::MSG_PVS), Some(origin), ptr::null_mut()),
            Self::VisibleReliable(origin) => (code(sys::MSG_PVS_R), Some(origin), ptr::null_mut()),
            Self::Audible(origin) => (code(sys::MSG_PAS), Some(origin), ptr::null_mut()),
            Self::AudibleReliable(origin) => (code(sys::MSG_PAS_R), Some(origin), ptr::null_mut()),
            Self::Init => (code(sys::MSG_INIT), None, ptr::null_mut()),
            Self::Spectators => (code(sys::MSG_SPEC), None, ptr::null_mut()),
        }
    }
}

/// A message type: a user message from [`Engine::user_message`] or an
/// engine `svc_*` number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageType(pub i32);

impl MessageType {
    /// `svc_temp_entity`, for `TE_*` effects.
    pub const TEMP_ENTITY: Self = Self(sys::SVC_TEMPENTITY as i32);
}

/// [`Engine::message`] refused to start a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageError {
    /// Another [`MessageWriter`] is still open; the engine allows one
    /// message at a time.
    AlreadyWriting,
}

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("another message is still being written")
    }
}

impl Error for MessageError {}

/// Writes the body of one network message. Dropping it sends the message
/// (`MESSAGE_END`), also when a panic unwinds through it.
#[must_use = "the message is sent when the writer is dropped"]
pub struct MessageWriter<'engine> {
    engine: &'engine Engine,
}

impl Engine {
    /// Looks up a user message that the game registered, such as `SayText`.
    pub fn user_message(&self, name: &CStr) -> Option<MessageType> {
        let mut size = 0;
        // SAFETY: metamod reads the name and writes the size during the call.
        let id = unsafe {
            host_call!(
                self.util,
                pfnGetUserMsgID(self.plugin_id, name.as_ptr(), &mut size)
            )
        };
        (id != 0).then_some(MessageType(id))
    }

    /// Starts a message (`MESSAGE_BEGIN`).
    pub fn message(
        &self,
        destination: MessageDestination,
        kind: MessageType,
    ) -> Result<MessageWriter<'_>, MessageError> {
        if self.writing_message.replace(true) {
            return Err(MessageError::AlreadyWriting);
        }
        let (code, origin, receiver) = destination.raw();
        let origin = origin.map_or(ptr::null(), |origin| origin.as_ptr());
        // SAFETY: the engine copies the origin during the call and `receiver`
        // is null or an engine edict.
        unsafe {
            host_call!(
                self.functions,
                pfnMessageBegin(code, kind.0, origin, receiver)
            )
        }
        Ok(MessageWriter { engine: self })
    }
}

/// Calls one `Write*` engine function and returns the writer for chaining.
macro_rules! write_value {
    ($writer:ident, $function:ident($value:expr)) => {{
        // SAFETY: the function comes from the engine table and only reads its
        // argument during the call.
        unsafe { host_call!($writer.engine.functions, $function($value)) }
        $writer
    }};
}

impl MessageWriter<'_> {
    pub fn byte(&mut self, value: u8) -> &mut Self {
        write_value!(self, pfnWriteByte(value.into()))
    }

    pub fn char(&mut self, value: i8) -> &mut Self {
        write_value!(self, pfnWriteChar(value.into()))
    }

    pub fn short(&mut self, value: i16) -> &mut Self {
        write_value!(self, pfnWriteShort(value.into()))
    }

    /// A 16-bit value read back as unsigned, such as HUD fixed-point times.
    pub fn unsigned_short(&mut self, value: u16) -> &mut Self {
        write_value!(self, pfnWriteShort(value.into()))
    }

    pub fn long(&mut self, value: i32) -> &mut Self {
        write_value!(self, pfnWriteLong(value))
    }

    /// An angle in degrees, sent in one byte.
    pub fn angle(&mut self, degrees: f32) -> &mut Self {
        write_value!(self, pfnWriteAngle(degrees))
    }

    /// A world coordinate.
    pub fn coord(&mut self, value: f32) -> &mut Self {
        write_value!(self, pfnWriteCoord(value))
    }

    pub fn string(&mut self, value: &CStr) -> &mut Self {
        write_value!(self, pfnWriteString(value.as_ptr()))
    }

    /// An entity index.
    pub fn entity(&mut self, entity: Edict) -> &mut Self {
        write_value!(self, pfnWriteEntity(entity.index()))
    }
}

impl Drop for MessageWriter<'_> {
    fn drop(&mut self) {
        // SAFETY: ends the message that `Engine::message` began.
        unsafe { host_call!(self.engine.functions, pfnMessageEnd()) }
        self.engine.writing_message.set(false);
    }
}
