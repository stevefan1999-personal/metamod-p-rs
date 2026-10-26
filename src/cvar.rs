use std::{
    cell::UnsafeCell,
    error::Error,
    ffi::{c_int, CStr},
    fmt,
    ptr::{self, addr_of, NonNull},
};

use crate::{
    engine::host_call,
    ffi::{c_str, to_c_string},
    sys::{self, cvar_t},
    Engine, MainThreadCell,
};

/// Cvar flags (`FCVAR_*`). Combine them with [`CvarFlags::union`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CvarFlags(c_int);

impl CvarFlags {
    /// Saved to the config file.
    pub const ARCHIVE: Self = Self(sys::FCVAR_ARCHIVE as c_int);
    /// Owned by a game DLL or plugin.
    pub const EXTDLL: Self = Self(sys::FCVAR_EXTDLL as c_int);
    /// The value is hidden from server queries.
    pub const PROTECTED: Self = Self(sys::FCVAR_PROTECTED as c_int);
    /// Changes are announced to the players.
    pub const SERVER: Self = Self(sys::FCVAR_SERVER as c_int);
    /// Changes are not written to the server log.
    pub const UNLOGGED: Self = Self(sys::FCVAR_UNLOGGED as c_int);

    /// Both sets of flags; usable in a `static`.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A cvar definition in a `static`, as the engine requires.
///
/// The engine keeps a pointer to the definition, so it must never move.
/// Metamod registers a copy of it instead, which is why values are read
/// back through the [`Cvar`] that [`StaticCvar::register`] returns rather
/// than from this definition.
pub struct StaticCvar {
    name:       &'static CStr,
    definition: MainThreadCell<UnsafeCell<cvar_t>>,
}

impl StaticCvar {
    pub const fn new(name: &'static CStr, default: &'static CStr, flags: CvarFlags) -> Self {
        Self {
            name,
            definition: MainThreadCell::new(UnsafeCell::new(cvar_t {
                name:   name.as_ptr().cast_mut(),
                string: default.as_ptr().cast_mut(),
                flags:  flags.0,
                value:  0.0,
                next:   ptr::null_mut(),
            })),
        }
    }

    /// Registers the cvar and returns the engine's live copy of it.
    pub fn register(&'static self, engine: &'static Engine) -> Result<Cvar, CvarError> {
        // SAFETY: the definition is static, so it outlives every pointer the
        // engine or metamod keeps to it.
        unsafe {
            host_call!(
                engine.functions,
                pfnCVarRegister(self.definition.get().get())
            )
        }
        engine.cvar(self.name).ok_or(CvarError { name: self.name })
    }
}

/// A live engine cvar, such as one from [`StaticCvar::register`] or
/// [`Engine::cvar`].
#[derive(Clone, Copy)]
pub struct Cvar {
    raw:    NonNull<cvar_t>,
    engine: &'static Engine,
}

impl Cvar {
    /// Cvar name.
    pub fn name(&self) -> &CStr {
        // SAFETY: registered cvars keep their name for the whole process.
        unsafe { c_str(addr_of!((*self.raw.as_ptr()).name).read()) }.unwrap_or_default()
    }

    /// Current value as a number.
    pub fn value(&self) -> f32 {
        // SAFETY: registered cvars live for the whole process.
        unsafe { addr_of!((*self.raw.as_ptr()).value).read() }
    }

    /// Current value as text.
    pub fn string(&self) -> String {
        // SAFETY: the engine replaces the string on every change, so it is
        // copied out right away.
        unsafe { c_str(addr_of!((*self.raw.as_ptr()).string).read()) }
            .map(|text| text.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Sets the value through the engine, which also updates the text and
    /// announces `SERVER` cvars.
    pub fn set_value(&self, value: f32) {
        // SAFETY: the name is a C string owned by the engine.
        unsafe {
            host_call!(
                self.engine.functions,
                pfnCVarSetFloat(self.name().as_ptr(), value)
            )
        }
    }

    /// Sets the text through the engine, which also updates the value.
    pub fn set_string(&self, value: &str) {
        let value = to_c_string(value);
        // SAFETY: both are C strings that outlive the call.
        unsafe {
            host_call!(
                self.engine.functions,
                pfnCVarSetString(self.name().as_ptr(), value.as_ptr())
            )
        }
    }
}

impl fmt::Debug for Cvar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cvar")
            .field("name", &self.name())
            .field("string", &self.string())
            .finish()
    }
}

impl Engine {
    /// Looks up a registered cvar, such as `sv_gravity`.
    pub fn cvar(&'static self, name: &CStr) -> Option<Cvar> {
        // SAFETY: CVarGetPointer returns null or a cvar the engine never frees.
        let raw = unsafe { host_call!(self.functions, pfnCVarGetPointer(name.as_ptr())) };
        NonNull::new(raw).map(|raw| Cvar { raw, engine: self })
    }
}

/// The engine has no cvar under the registered name, for example because
/// the name was already taken by a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvarError {
    name: &'static CStr,
}

impl fmt::Display for CvarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cvar {:?} is missing after registration",
            self.name
        )
    }
}

impl Error for CvarError {}
