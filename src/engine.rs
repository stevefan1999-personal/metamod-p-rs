use std::{
    cell::{Cell, RefCell},
    ffi::{c_char, CStr},
    ptr::{addr_of, NonNull},
};

use crate::{
    command::Command,
    ffi::{c_str, to_c_string},
    sys::{enginefuncs_t, globalvars_t, mutil_funcs_t, plid_t, string_t, ALERT_TYPE, PRINT_TYPE},
    Edict,
};

/// Calls a function from a host table. Metamod fills every entry, so an
/// empty one means a broken host: it panics, and the FFI guard logs it.
macro_rules! host_call {
    ($table:expr, $function:ident($($argument:expr),* $(,)?)) => {
        ($table
            .$function
            .expect(concat!(stringify!($function), " is missing from the host table")))(
            $($argument),*
        )
    };
}
pub(crate) use host_call;

/// The engine and metamod functions a plugin can call.
///
/// [`Plugin::attach`](crate::Plugin::attach) receives it; it stays valid for
/// as long as the plugin is loaded. Like the engine itself it is
/// single-threaded: it is neither `Send` nor `Sync`.
pub struct Engine {
    pub(crate) functions:       enginefuncs_t,
    pub(crate) globals:         NonNull<globalvars_t>,
    pub(crate) util:            mutil_funcs_t,
    pub(crate) plugin_id:       plid_t,
    pub(crate) commands:        RefCell<Vec<Command>>,
    pub(crate) writing_message: Cell<bool>,
}

/// Where [`Engine::alert`] sends its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    /// `at_notice`.
    Notice,
    /// `at_console`: shown when `developer` is at least 1.
    Console,
    /// `at_aiconsole`: shown when `developer` is at least 2.
    AiConsole,
    /// `at_warning`.
    Warning,
    /// `at_error`.
    Error,
    /// `at_logged`: printed and written to the server log.
    Logged,
}

impl AlertKind {
    const fn raw(self) -> ALERT_TYPE {
        match self {
            Self::Notice => ALERT_TYPE::at_notice,
            Self::Console => ALERT_TYPE::at_console,
            Self::AiConsole => ALERT_TYPE::at_aiconsole,
            Self::Warning => ALERT_TYPE::at_warning,
            Self::Error => ALERT_TYPE::at_error,
            Self::Logged => ALERT_TYPE::at_logged,
        }
    }
}

/// Where [`Engine::client_print`] shows its text on the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrintKind {
    /// The client's console.
    Console,
    /// The middle of the screen.
    Center,
    /// The chat area.
    Chat,
}

impl PrintKind {
    const fn raw(self) -> PRINT_TYPE {
        match self {
            Self::Console => PRINT_TYPE::print_console,
            Self::Center => PRINT_TYPE::print_center,
            Self::Chat => PRINT_TYPE::print_chat,
        }
    }
}

impl Engine {
    /// Builds an engine handle from raw host tables.
    ///
    /// # Safety
    ///
    /// Every function in `functions` and `util` must be a valid host
    /// function, `globals` must point to the engine globals and `plugin_id`
    /// to this plugin's info, both for as long as the handle is used.
    pub unsafe fn from_raw(
        functions: enginefuncs_t,
        globals: NonNull<globalvars_t>,
        util: mutil_funcs_t,
        plugin_id: plid_t,
    ) -> Self {
        Self {
            functions,
            globals,
            util,
            plugin_id,
            commands: RefCell::default(),
            writing_message: Cell::default(),
        }
    }

    /// The raw engine function table, for calls the safe API does not wrap.
    pub fn functions(&self) -> &enginefuncs_t {
        &self.functions
    }

    /// The raw metamod utility table.
    pub fn util_functions(&self) -> &mutil_funcs_t {
        &self.util
    }

    /// The plugin id that the metamod utility functions expect.
    pub fn plugin_id(&self) -> plid_t {
        self.plugin_id
    }

    /// The engine's global variables.
    pub fn globals(&self) -> NonNull<globalvars_t> {
        self.globals
    }

    /// Prints `text` to the server console as is; add `\n` yourself.
    pub fn server_print(&self, text: &str) {
        let text = to_c_string(text);
        // SAFETY: `text` is a C string that outlives the call.
        unsafe { host_call!(self.functions, pfnServerPrint(text.as_ptr())) }
    }

    /// Sends `text` through the engine's alert channel.
    pub fn alert(&self, kind: AlertKind, text: &str) {
        let text = to_c_string(text);
        // SAFETY: the text goes in as a `%s` argument, not as the format, so
        // a `%` in it is printed instead of read as a conversion.
        unsafe {
            host_call!(
                self.functions,
                pfnAlertMessage(kind.raw(), c"%s".as_ptr().cast_mut(), text.as_ptr())
            )
        }
    }

    /// Prints `text` on one client.
    ///
    /// # Panics
    ///
    /// Panics when `player` is not valid.
    pub fn client_print(&self, player: Edict, kind: PrintKind, text: &str) {
        let text = to_c_string(text);
        // SAFETY: `player` is an edict of the current array; the engine
        // ignores non-clients.
        unsafe {
            host_call!(
                self.functions,
                pfnClientPrintf(player.checked_ptr(), kind.raw(), text.as_ptr())
            )
        }
    }

    /// Queues `command` on the server console as if an admin typed it. It
    /// runs on the next frame, or now after [`Engine::server_execute`].
    pub fn server_command(&self, command: &str) {
        let line = to_c_string(&format!("{command}\n"));
        // SAFETY: the engine copies the line into its command buffer.
        unsafe { host_call!(self.functions, pfnServerCommand(line.as_ptr().cast_mut())) }
    }

    /// Runs the queued server console commands now.
    ///
    /// # Safety
    ///
    /// The queued commands must not unload or reload this plugin, such as
    /// `meta unload`, `meta reload` or `meta force_unload`: metamod would
    /// close the library while this call is on its stack. The plugin refuses
    /// the first two while its code runs, but `force_unload` ignores that.
    pub unsafe fn server_execute(&self) {
        // SAFETY: takes no arguments; the caller keeps the plugin loaded.
        unsafe { host_call!(self.functions, pfnServerExecute()) }
    }

    /// Runs `command` in a client's console (`stufftext`). Never pass
    /// unchecked player input: the client executes every command in it.
    ///
    /// # Panics
    ///
    /// Panics when `player` is not valid.
    pub fn client_command(&self, player: Edict, command: &str) {
        let line = to_c_string(&format!("{command}\n"));
        // SAFETY: `%s` keeps the line out of the format string; `player` is
        // an edict of the current array and the engine ignores non-clients.
        unsafe {
            host_call!(
                self.functions,
                pfnClientCommand(
                    player.checked_ptr(),
                    c"%s".as_ptr().cast_mut(),
                    line.as_ptr()
                )
            )
        }
    }

    /// Precaches a model and returns its model index.
    ///
    /// The engine keeps the path pointer for the whole map, and a pointer
    /// into the plugin would dangle after `meta unload`. So the path is
    /// copied into the engine's string pool first, which lives as long as
    /// the map. GoldSrc reads backslashes in pooled strings as escapes: use
    /// `/` in paths.
    pub fn precache_model(&self, path: &CStr) -> i32 {
        // SAFETY: the pooled copy lives as long as the precache table.
        unsafe { host_call!(self.functions, pfnPrecacheModel(self.pooled(path))) }
    }

    /// Precaches a sound and returns its index; see [`Engine::precache_model`].
    pub fn precache_sound(&self, path: &CStr) -> i32 {
        // SAFETY: the pooled copy lives as long as the precache table.
        unsafe { host_call!(self.functions, pfnPrecacheSound(self.pooled(path))) }
    }

    /// Adds a file the clients must download; see [`Engine::precache_model`].
    pub fn precache_generic(&self, path: &CStr) -> i32 {
        // SAFETY: the pooled copy lives as long as the precache table.
        unsafe { host_call!(self.functions, pfnPrecacheGeneric(self.pooled(path))) }
    }

    /// Copies `text` into the engine's string pool, which the engine frees
    /// with the map, and returns the engine's copy.
    fn pooled(&self, text: &CStr) -> *mut c_char {
        // SAFETY: AllocString copies the text and SzFromIndex returns that
        // copy, which the engine owns.
        unsafe {
            let index = host_call!(self.functions, pfnAllocString(text.as_ptr()));
            host_call!(self.functions, pfnSzFromIndex(index)).cast_mut()
        }
    }

    /// Seconds since the map started.
    pub fn time(&self) -> f32 {
        // SAFETY: `globals` points to the engine globals while loaded.
        unsafe { addr_of!((*self.globals.as_ptr()).time).read() }
    }

    /// Number of player slots on this server.
    pub fn max_clients(&self) -> i32 {
        // SAFETY: `globals` points to the engine globals while loaded.
        unsafe { addr_of!((*self.globals.as_ptr()).maxClients).read() }
    }

    /// Name of the current map, such as `hl_c01_a1`.
    pub fn map_name(&self) -> Option<String> {
        // SAFETY: `globals` points to the engine globals while loaded, and
        // the engine keeps `mapname` as one of its strings.
        unsafe { self.string(addr_of!((*self.globals.as_ptr()).mapname).read()) }
    }

    /// Copies an engine string (`string_t`), or `None` for the null string.
    ///
    /// # Safety
    ///
    /// `value` must be a `string_t` that the engine or the game made, such
    /// as an entvars field. It is an unchecked offset into the engine's
    /// string table, so any other value reads arbitrary memory.
    pub unsafe fn string(&self, value: string_t) -> Option<String> {
        if value == 0 {
            return None;
        }
        // SAFETY: for an engine string the engine returns a pointer into its
        // string table; the text is copied before any other engine call can
        // change the table.
        unsafe { c_str(host_call!(self.functions, pfnSzFromIndex(value))) }
            .map(|text| text.to_string_lossy().into_owned())
    }
}
