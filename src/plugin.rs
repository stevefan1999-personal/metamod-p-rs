use std::{
    error::Error,
    ffi::{c_char, c_int, CStr},
};

use crate::{
    export::Exported,
    hooks::{dispatch, dispatch_post, give_table, plugin_trait, FromRaw, HookReturn},
    sys::{
        self, edict_t, enginefuncs_t, plugin_info_t, DLL_FUNCTIONS, NEW_DLL_FUNCTIONS,
        PLUG_LOADTIME, PL_UNLOAD_REASON,
    },
    Edict, Engine, MessageType, MetaResult, PostResult, Returned,
};

/// What `meta list` and `meta info` show about a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginInfo {
    pub name:       &'static CStr,
    pub version:    &'static CStr,
    pub date:       &'static CStr,
    pub author:     &'static CStr,
    pub url:        &'static CStr,
    /// Prefix of the plugin's log lines.
    pub log_tag:    &'static CStr,
    /// When metamod may load the plugin.
    pub loadable:   LoadTime,
    /// When metamod may unload the plugin.
    pub unloadable: LoadTime,
}

impl PluginInfo {
    pub(crate) const fn to_raw(self) -> plugin_info_t {
        plugin_info_t {
            ifvers:     sys::META_INTERFACE_VERSION.as_ptr().cast_mut(),
            name:       self.name.as_ptr().cast_mut(),
            version:    self.version.as_ptr().cast_mut(),
            date:       self.date.as_ptr().cast_mut(),
            author:     self.author.as_ptr().cast_mut(),
            url:        self.url.as_ptr().cast_mut(),
            logtag:     self.log_tag.as_ptr().cast_mut(),
            loadable:   self.loadable.to_raw(),
            unloadable: self.unloadable.to_raw(),
        }
    }
}

/// A point in the server's life at which metamod loads or unloads plugins.
/// Later variants allow more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LoadTime {
    Never,
    /// Only when the server starts.
    Startup,
    /// Between maps.
    ChangeLevel,
    /// At any time.
    AnyTime,
    /// At any time, and the plugin may be paused during a map.
    AnyPause,
}

impl LoadTime {
    const ALL: [Self; 5] = [
        Self::Never,
        Self::Startup,
        Self::ChangeLevel,
        Self::AnyTime,
        Self::AnyPause,
    ];

    pub(crate) const fn to_raw(self) -> PLUG_LOADTIME {
        match self {
            Self::Never => PLUG_LOADTIME::PT_NEVER,
            Self::Startup => PLUG_LOADTIME::PT_STARTUP,
            Self::ChangeLevel => PLUG_LOADTIME::PT_CHANGELEVEL,
            Self::AnyTime => PLUG_LOADTIME::PT_ANYTIME,
            Self::AnyPause => PLUG_LOADTIME::PT_ANYPAUSE,
        }
    }

    pub(crate) fn from_raw(raw: PLUG_LOADTIME) -> Option<Self> {
        Self::ALL.into_iter().find(|time| time.to_raw() == raw)
    }
}

/// Why metamod unloads a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnloadReason {
    /// Metamod gave no reason.
    Unspecified,
    /// The plugin was removed from `plugins.ini`.
    IniDeleted,
    /// The plugin file on disk is newer than the loaded one.
    FileNewer,
    /// A console command asked for it.
    Command,
    /// A console command forced it.
    CommandForced,
    /// An earlier, delayed request.
    Delayed,
    /// Another plugin asked for it.
    Plugin,
    /// Another plugin forced it.
    PluginForced,
    /// `meta reload`.
    Reload,
    /// The server shuts down (metamod-svencoop).
    Shutdown,
    /// A reason this crate does not know.
    Other(u32),
}

impl UnloadReason {
    pub(crate) fn from_raw(raw: PL_UNLOAD_REASON) -> Self {
        match raw {
            PL_UNLOAD_REASON::PNL_NULL => Self::Unspecified,
            PL_UNLOAD_REASON::PNL_INI_DELETED => Self::IniDeleted,
            PL_UNLOAD_REASON::PNL_FILE_NEWER => Self::FileNewer,
            PL_UNLOAD_REASON::PNL_COMMAND => Self::Command,
            PL_UNLOAD_REASON::PNL_CMD_FORCED => Self::CommandForced,
            PL_UNLOAD_REASON::PNL_DELAYED => Self::Delayed,
            PL_UNLOAD_REASON::PNL_PLUGIN => Self::Plugin,
            PL_UNLOAD_REASON::PNL_PLG_FORCED => Self::PluginForced,
            PL_UNLOAD_REASON::PNL_RELOAD => Self::Reload,
            #[cfg(feature = "fallguys")]
            PL_UNLOAD_REASON::PNL_SHUTDOWN => Self::Shutdown,
            other => Self::Other(other.0),
        }
    }
}

/// The hook methods that a [`Plugin`] impl overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overrides {
    /// Install every hook. This is correct, but metamod then calls the
    /// plugin for every hooked function; `#[metamod_p::plugin]` avoids that.
    All,
    /// Install only the hook methods with these names.
    Only(&'static [&'static str]),
}

impl Overrides {
    pub(crate) fn contains(&self, hook: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(hooks) => hooks.contains(&hook),
        }
    }
}

/// The engine's buffer for the message that a rejected client sees.
#[repr(transparent)]
pub struct RejectReason([c_char; RejectReason::CAPACITY]);

impl RejectReason {
    /// Buffer size in bytes, including the terminating NUL.
    pub const CAPACITY: usize = 128;

    /// The current message.
    pub fn get(&self) -> &CStr {
        // SAFETY: `c_char` and `u8` have the same size and alignment.
        let bytes =
            unsafe { std::slice::from_raw_parts(self.0.as_ptr().cast::<u8>(), Self::CAPACITY) };
        CStr::from_bytes_until_nul(bytes).unwrap_or_default()
    }

    /// Replaces the message, cut on a character boundary to fit the buffer.
    pub fn set(&mut self, reason: &str) {
        let text: String = reason
            .chars()
            .filter(|&character| character != '\0')
            .collect();
        let length = (0..=text.len().min(Self::CAPACITY - 1))
            .rev()
            .find(|&end| text.is_char_boundary(end))
            .unwrap_or_default();
        let terminated = text.as_bytes()[..length].iter().copied().chain([0]);
        self.0
            .iter_mut()
            .zip(terminated)
            .for_each(|(slot, byte)| *slot = byte as c_char);
    }
}

impl FromRaw<c_int> for MessageType {
    unsafe fn from_raw(raw: c_int, _: &'static Engine) -> Option<Self> {
        Some(Self(raw))
    }
}

plugin_trait! {
    /// A metamod plugin.
    ///
    /// Write an `impl Plugin for YourType` with `#[metamod_p::plugin]` on
    /// it: the attribute exports the C entry points that metamod looks for
    /// and installs only the hooks the impl overrides.
    ///
    /// Pre hooks return a [`MetaResult`] and default to
    /// [`MetaResult::Ignored`]. Each has a `_post` twin that runs after the
    /// real function and returns a [`PostResult`]; post hooks of functions
    /// with a return value also get the [`Returned`] values. Hooks take
    /// `&self` because the engine calls back into plugins while a hook
    /// runs: keep mutable state in `Cell` or `RefCell` fields.
    pub trait Plugin {
        /// What `meta list` and `meta info` show.
        const INFO: PluginInfo;

        /// The hook methods this impl overrides; `#[metamod_p::plugin]`
        /// fills it in.
        const OVERRIDES: Overrides = Overrides::All;

        /// Called from `Meta_Query` with metamod's interface version, such as
        /// `5:16`, once the crate has checked that metamod is not older than
        /// the headers. Return an error to refuse to load.
        #[allow(unused_variables)]
        fn query(metamod_interface: &CStr) -> Result<(), Box<dyn Error>> {
            Ok(())
        }

        /// Called from `Meta_Attach` to build the plugin. `engine` stays valid
        /// while the plugin is loaded. Return an error to refuse to load.
        fn attach(engine: &'static Engine, now: LoadTime) -> Result<Self, Box<dyn Error>>;

        /// Called from `Meta_Detach`; dropping `self` releases the plugin.
        #[allow(unused_variables)]
        fn detach(self, now: LoadTime, reason: UnloadReason) {}
    }

    /// Hooks on the game DLL functions (`DLL_FUNCTIONS`).
    mod dll_hooks(DLL_FUNCTIONS, sys::INTERFACE_VERSION) {
        /// The game DLL initialises, once per server start.
        fn game_init / game_init_post() => pfnGameInit();
        /// An entity spawns. The game returns a negative value to delete it.
        fn spawn / spawn_post(entity: Edict) -> i32 => pfnSpawn(entity: *mut edict_t);
        /// An entity thinks.
        fn think / think_post(entity: Edict) => pfnThink(entity: *mut edict_t);
        /// `user` uses `used`, such as a player pressing a button.
        fn use_entity / use_entity_post(used: Edict, user: Edict)
            => pfnUse(used: *mut edict_t, user: *mut edict_t);
        /// Two entities touch.
        fn touch / touch_post(touched: Edict, other: Edict)
            => pfnTouch(touched: *mut edict_t, other: *mut edict_t);
        /// `other` blocks a moving `blocked` entity.
        fn blocked / blocked_post(blocked: Edict, other: Edict)
            => pfnBlocked(blocked: *mut edict_t, other: *mut edict_t);
        /// A client connects. Return `false` (with `Supercede`) and set the
        /// reject reason to refuse it.
        fn client_connect / client_connect_post(
            player: Edict,
            name: &CStr,
            address: &CStr,
            reject_reason: &mut RejectReason,
        ) -> bool => pfnClientConnect(
            player: *mut edict_t,
            name: *const c_char,
            address: *const c_char,
            reject_reason: *mut [c_char; RejectReason::CAPACITY],
        );
        /// A client disconnects.
        fn client_disconnect / client_disconnect_post(player: Edict)
            => pfnClientDisconnect(player: *mut edict_t);
        /// A player uses the `kill` command.
        fn client_kill / client_kill_post(player: Edict) => pfnClientKill(player: *mut edict_t);
        /// A client enters the game.
        fn client_put_in_server / client_put_in_server_post(player: Edict)
            => pfnClientPutInServer(player: *mut edict_t);
        /// A client sends a command; read it with [`Engine::command_args`].
        fn client_command / client_command_post(player: Edict)
            => pfnClientCommand(player: *mut edict_t);
        /// A client changes its user info. `info` is the engine's buffer of
        /// `\key\value` pairs. It is a raw pointer because the usual job of
        /// this hook, `pfnSetClientKeyValue` through [`Engine::functions`],
        /// writes the buffer during the call; read keys with `pfnInfoKeyValue`.
        fn client_user_info_changed / client_user_info_changed_post(
            player: Edict,
            info: *mut c_char,
        ) => pfnClientUserInfoChanged(player: *mut edict_t, info: *mut c_char);
        /// A map has loaded and spawned its entities.
        fn server_activate / server_activate_post(edict_count: i32, max_clients: i32)
            => pfnServerActivate(_edicts: *mut edict_t, edict_count: c_int, max_clients: c_int);
        /// The map is about to end.
        fn server_deactivate / server_deactivate_post() => pfnServerDeactivate();
        /// A player is about to think.
        fn player_pre_think / player_pre_think_post(player: Edict)
            => pfnPlayerPreThink(player: *mut edict_t);
        /// A player has thought.
        fn player_post_think / player_post_think_post(player: Edict)
            => pfnPlayerPostThink(player: *mut edict_t);
        /// A server frame starts.
        fn start_frame / start_frame_post() => pfnStartFrame();
    }

    /// Hooks on the extra game DLL functions (`NEW_DLL_FUNCTIONS`).
    mod new_dll_hooks(NEW_DLL_FUNCTIONS, sys::NEW_DLL_FUNCTIONS_VERSION) {
        /// The engine is about to free an entity.
        fn free_entity / free_entity_post(entity: Edict)
            => pfnOnFreeEntPrivateData(entity: *mut edict_t);
        /// The game DLL shuts down.
        fn game_shutdown / game_shutdown_post() => pfnGameShutdown();
        /// The engine asks whether two entities collide.
        fn should_collide / should_collide_post(touched: Edict, other: Edict) -> bool
            => pfnShouldCollide(touched: *mut edict_t, other: *mut edict_t);
        /// A client answered a cvar query.
        fn cvar_value / cvar_value_post(player: Edict, request_id: i32, name: &CStr, value: &CStr)
            => pfnCvarValue2(
                player: *const edict_t,
                request_id: c_int,
                name: *const c_char,
                value: *const c_char,
            );
    }

    /// Hooks on the engine functions (`enginefuncs_t`) that the game DLL and
    /// other plugins call. Never add `pfnPEntityOfEntIndexAllEntities`: it is
    /// only in the Metamod-P headers and past the end of metamod-svencoop's
    /// table.
    mod engine_hooks(enginefuncs_t, sys::ENGINE_INTERFACE_VERSION) {
        /// Someone precaches a model.
        fn precache_model / precache_model_post(path: &CStr) -> i32
            => pfnPrecacheModel(path: *mut c_char);
        /// Someone precaches a sound.
        fn precache_sound / precache_sound_post(path: &CStr) -> i32
            => pfnPrecacheSound(path: *mut c_char);
        /// Someone sets an entity's model.
        fn set_model / set_model_post(entity: Edict, model: &CStr)
            => pfnSetModel(entity: *mut edict_t, model: *const c_char);
        /// Someone plays a sound on an entity.
        fn emit_sound / emit_sound_post(
            entity: Edict,
            channel: i32,
            sample: &CStr,
            volume: f32,
            attenuation: f32,
            flags: i32,
            pitch: i32,
        ) => pfnEmitSound(
            entity: *mut edict_t,
            channel: c_int,
            sample: *const c_char,
            volume: f32,
            attenuation: f32,
            flags: c_int,
            pitch: c_int,
        );
        /// Someone sets a player's speed limit.
        fn set_client_max_speed / set_client_max_speed_post(player: Edict, speed: f32)
            => pfnSetClientMaxspeed(player: *const edict_t, speed: f32);
        /// The game registers a user message and gets its id back.
        fn register_user_message / register_user_message_post(name: &CStr, size: i32) -> i32
            => pfnRegUserMsg(name: *const c_char, size: c_int);
        /// Someone starts a network message. `destination` is a `MSG_*`
        /// code.
        fn message_begin / message_begin_post(
            destination: i32,
            kind: MessageType,
            origin: Option<[f32; 3]>,
            receiver: Option<Edict>,
        ) => pfnMessageBegin(
            destination: c_int,
            kind: c_int,
            origin: *const f32,
            receiver: *mut edict_t,
        );
        /// Someone ends a network message.
        fn message_end / message_end_post() => pfnMessageEnd();
        /// A message byte.
        fn write_byte / write_byte_post(value: i32) => pfnWriteByte(value: c_int);
        /// A message char.
        fn write_char / write_char_post(value: i32) => pfnWriteChar(value: c_int);
        /// A message short.
        fn write_short / write_short_post(value: i32) => pfnWriteShort(value: c_int);
        /// A message long.
        fn write_long / write_long_post(value: i32) => pfnWriteLong(value: c_int);
        /// A message angle in degrees.
        fn write_angle / write_angle_post(value: f32) => pfnWriteAngle(value: f32);
        /// A message coordinate.
        fn write_coord / write_coord_post(value: f32) => pfnWriteCoord(value: f32);
        /// A message string.
        fn write_string / write_string_post(value: &CStr)
            => pfnWriteString(value: *const c_char);
        /// A message entity index.
        fn write_entity / write_entity_post(value: i32) => pfnWriteEntity(value: c_int);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reject_reason() -> RejectReason {
        RejectReason([0; RejectReason::CAPACITY])
    }

    #[test]
    fn reject_reason_round_trips_short_text() {
        let mut reason = reject_reason();
        reason.set("Server is full");
        assert_eq!(reason.get(), c"Server is full");
    }

    #[test]
    fn reject_reason_is_cut_on_a_character_boundary() {
        let mut reason = reject_reason();
        // 126 ASCII bytes and a 2-byte character do not fit in 127 bytes.
        let text = format!("{}é", "a".repeat(RejectReason::CAPACITY - 2));
        reason.set(&text);
        assert_eq!(
            reason.get().to_bytes(),
            "a".repeat(RejectReason::CAPACITY - 2).as_bytes()
        );
    }

    #[test]
    fn reject_reason_drops_interior_nul() {
        let mut reason = reject_reason();
        reason.set("no\0pe");
        assert_eq!(reason.get(), c"nope");
    }

    #[test]
    fn load_times_round_trip_through_metamod_codes() {
        for time in LoadTime::ALL {
            assert_eq!(LoadTime::from_raw(time.to_raw()), Some(time));
        }
        assert_eq!(LoadTime::from_raw(PLUG_LOADTIME(99)), None);
    }

    #[test]
    fn unknown_unload_reason_keeps_its_code() {
        assert_eq!(
            UnloadReason::from_raw(PL_UNLOAD_REASON::PNL_RELOAD),
            UnloadReason::Reload
        );
        assert_eq!(
            UnloadReason::from_raw(PL_UNLOAD_REASON(42)),
            UnloadReason::Other(42)
        );
    }

    #[test]
    fn overrides_only_contain_listed_hooks() {
        let only = Overrides::Only(&["client_connect"]);
        assert!(only.contains("client_connect"));
        assert!(!only.contains("think"));
        assert!(Overrides::All.contains("think"));
    }
}
