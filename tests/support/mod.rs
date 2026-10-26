//! A fake engine and metamod that record what a plugin calls, so the safe
//! layer can be driven through its real C entry points on the host.

use std::{
    cell::{Cell, RefCell},
    ffi::{c_char, c_int, CStr, CString},
    mem, ptr,
};

use metamod_p::sys::{
    cvar_t, edict_t, enginefuncs_t, gamedll_funcs_t, globalvars_t, meta_globals_t, mutil_funcs_t,
    plid_t, plugin_info_t, ALERT_TYPE, DLL_FUNCTIONS, FL_CLIENT, INTERFACE_VERSION, META_FUNCTIONS,
    NEW_DLL_FUNCTIONS, PLUG_LOADTIME, PRINT_TYPE,
};

pub const MAX_CLIENTS: c_int = 4;
pub const EDICT_COUNT: usize = 16;
pub const SAY_TEXT: c_int = 76;
pub const PRECACHED_MODEL_INDEX: c_int = 3;
pub const CONNECTED_PLAYER: c_int = 1;
pub const EMPTY_PLAYER_SLOT: c_int = 2;
pub const FREE_SLOT: c_int = 5;
/// The engine's string table: `string_t` values are offsets into it.
const STRINGS: &[u8] = b"\0hl_c01_a1\0";
const MAP_NAME: c_int = 1;

/// One call that the plugin made into the fake host.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    ServerPrint(String),
    Alert {
        kind:   u32,
        format: String,
        text:   String,
    },
    ClientPrint {
        player: c_int,
        kind:   u32,
        text:   String,
    },
    ServerCommand(String),
    ClientCommand {
        player: c_int,
        format: String,
        text:   String,
    },
    AllocString(String),
    PrecacheModel {
        path:   String,
        /// Whether the engine got a string from its own pool.
        pooled: bool,
    },
    CvarRegister {
        name:   String,
        string: String,
        flags:  c_int,
    },
    CvarSetFloat(String, f32),
    CvarSetString(String, String),
    AddServerCommand(String),
    MessageBegin {
        destination: c_int,
        kind:        c_int,
        origin:      Option<[f32; 3]>,
        receiver:    Option<c_int>,
    },
    WriteByte(c_int),
    WriteShort(c_int),
    WriteString(String),
    WriteEntity(c_int),
    MessageEnd,
    Log {
        function: &'static str,
        format:   String,
        text:     String,
    },
}

thread_local! {
    static CALLS: RefCell<Vec<Call>> = RefCell::default();
    static EDICTS: Cell<*mut edict_t> = const { Cell::new(ptr::null_mut()) };
    static CVARS: RefCell<Vec<*mut cvar_t>> = RefCell::default();
    static COMMANDS: RefCell<Vec<(String, unsafe extern "C" fn())>> = RefCell::default();
    static WORDS: RefCell<Vec<CString>> = RefCell::default();
    static REST: RefCell<CString> = RefCell::default();
    /// Strings that `AllocString` added to the pool, numbered after `STRINGS`.
    static POOLED: RefCell<Vec<CString>> = RefCell::default();
}

/// Every call recorded so far.
pub fn calls() -> Vec<Call> {
    CALLS.with_borrow(Clone::clone)
}

/// Forgets the recorded calls.
pub fn clear_calls() {
    CALLS.with_borrow_mut(Vec::clear);
}

fn record(call: Call) {
    CALLS.with_borrow_mut(|calls| calls.push(call));
}

fn text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return "<null>".into();
    }
    // SAFETY: the plugin passes NUL-terminated strings.
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

fn leak(text: &str) -> *mut c_char {
    CString::new(text).expect("no NUL").into_raw()
}

/// The fake edict at `index`.
pub fn edict(index: c_int) -> *mut edict_t {
    let index = usize::try_from(index).expect("index");
    assert!(index < EDICT_COUNT, "edict index");
    // SAFETY: the array has EDICT_COUNT edicts.
    EDICTS.get().wrapping_add(index)
}

fn index_of(edict: *const edict_t) -> c_int {
    if edict.is_null() {
        return 0;
    }
    let offset = edict as usize - EDICTS.get() as usize;
    c_int::try_from(offset / mem::size_of::<edict_t>()).expect("index")
}

unsafe extern "C" fn server_print(message: *const c_char) {
    record(Call::ServerPrint(text(message)));
}

/// Metamod-P declares the format as `char *`, metamod-svencoop as `const char
/// *`.
#[cfg(feature = "fallguys")]
type AlertFormat = *const c_char;
#[cfg(not(feature = "fallguys"))]
type AlertFormat = *mut c_char;

unsafe extern "C" fn alert(kind: ALERT_TYPE, format: AlertFormat, mut arguments: ...) {
    // SAFETY: the plugin passes one C string after its `%s` format.
    let argument = unsafe { arguments.next_arg::<*const c_char>() };
    record(Call::Alert {
        kind:   kind.0,
        format: text(format),
        text:   text(argument),
    });
}

unsafe extern "C" fn client_print(player: *mut edict_t, kind: PRINT_TYPE, message: *const c_char) {
    record(Call::ClientPrint {
        player: index_of(player),
        kind:   kind.0,
        text:   text(message),
    });
}

unsafe extern "C" fn server_command(command: *mut c_char) {
    record(Call::ServerCommand(text(command)));
}

unsafe extern "C" fn client_command(player: *mut edict_t, format: *mut c_char, mut arguments: ...) {
    // SAFETY: the plugin passes one C string after its `%s` format.
    let argument = unsafe { arguments.next_arg::<*const c_char>() };
    record(Call::ClientCommand {
        player: index_of(player),
        format: text(format),
        text:   text(argument),
    });
}

unsafe extern "C" fn precache_model(path: *mut c_char) -> c_int {
    let pooled = POOLED.with_borrow(|pooled| pooled.iter().any(|copy| copy.as_ptr() == path));
    record(Call::PrecacheModel {
        path: text(path),
        pooled,
    });
    PRECACHED_MODEL_INDEX
}

unsafe extern "C" fn alloc_string(value: *const c_char) -> c_int {
    record(Call::AllocString(text(value)));
    POOLED.with_borrow_mut(|pooled| {
        pooled.push(CString::new(text(value)).expect("no NUL"));
        c_int::try_from(STRINGS.len() + pooled.len() - 1).expect("index")
    })
}

unsafe extern "C" fn string_from_index(offset: c_int) -> *const c_char {
    let offset = usize::try_from(offset).expect("offset");
    match offset.checked_sub(STRINGS.len()) {
        Some(index) => POOLED.with_borrow(|pooled| pooled[index].as_ptr()),
        None => STRINGS[offset..].as_ptr().cast(),
    }
}

unsafe extern "C" fn entity_of_index(index: c_int) -> *mut edict_t {
    if (0..EDICT_COUNT as c_int).contains(&index) {
        edict(index)
    } else {
        ptr::null_mut()
    }
}

unsafe extern "C" fn index_of_edict(edict: *const edict_t) -> c_int {
    index_of(edict)
}

/// The byte offset from the start of the current edict array, as the engine
/// computes it; far pointers give a negative offset.
unsafe extern "C" fn entity_offset(edict: *const edict_t) -> c_int {
    c_int::try_from(edict as isize - EDICTS.get() as isize).unwrap_or(c_int::MIN)
}

fn find_cvar(name: *const c_char) -> *mut cvar_t {
    let name = text(name);
    CVARS
        .with_borrow(|cvars| {
            // SAFETY: registered cvars are leaked copies.
            cvars
                .iter()
                .copied()
                .find(|&cvar| text(unsafe { (*cvar).name }) == name)
        })
        .unwrap_or(ptr::null_mut())
}

/// Registers a copy of the cvar, as metamod does.
unsafe extern "C" fn cvar_register(definition: *mut cvar_t) {
    // SAFETY: the plugin passes its static definition.
    let definition = unsafe { *definition };
    let (name, string) = (text(definition.name), text(definition.string));
    record(Call::CvarRegister {
        name:   name.clone(),
        string: string.clone(),
        flags:  definition.flags,
    });
    let copy = Box::leak(Box::new(cvar_t {
        name: leak(&name),
        string: leak(&string),
        value: string.parse().unwrap_or_default(),
        ..definition
    }));
    CVARS.with_borrow_mut(|cvars| cvars.push(copy));
}

unsafe extern "C" fn cvar_pointer(name: *const c_char) -> *mut cvar_t {
    find_cvar(name)
}

unsafe extern "C" fn cvar_set_float(name: *const c_char, value: f32) {
    record(Call::CvarSetFloat(text(name), value));
    // SAFETY: registered cvars are leaked copies.
    if let Some(cvar) = unsafe { find_cvar(name).as_mut() } {
        cvar.value = value;
        cvar.string = leak(&value.to_string());
    }
}

unsafe extern "C" fn cvar_set_string(name: *const c_char, value: *const c_char) {
    record(Call::CvarSetString(text(name), text(value)));
    // SAFETY: registered cvars are leaked copies.
    if let Some(cvar) = unsafe { find_cvar(name).as_mut() } {
        cvar.value = text(value).parse().unwrap_or_default();
        cvar.string = leak(&text(value));
    }
}

unsafe extern "C" fn add_server_command(
    name: *mut c_char,
    function: Option<unsafe extern "C" fn()>,
) {
    record(Call::AddServerCommand(text(name)));
    let function = function.expect("command callback");
    COMMANDS.with_borrow_mut(|commands| commands.push((text(name), function)));
}

unsafe extern "C" fn argument_count() -> c_int {
    WORDS.with_borrow(|words| c_int::try_from(words.len()).expect("count"))
}

unsafe extern "C" fn argument(index: c_int) -> *const c_char {
    WORDS.with_borrow(|words| {
        words
            .get(usize::try_from(index).expect("index"))
            .map_or(c"".as_ptr(), |word| word.as_ptr())
    })
}

unsafe extern "C" fn arguments() -> *const c_char {
    REST.with_borrow(|rest| rest.as_ptr())
}

unsafe extern "C" fn message_begin(
    destination: c_int,
    kind: c_int,
    origin: *const f32,
    receiver: *mut edict_t,
) {
    // SAFETY: a non-null origin points to three floats.
    let origin = (!origin.is_null()).then(|| unsafe { origin.cast::<[f32; 3]>().read() });
    let receiver = (!receiver.is_null()).then(|| index_of(receiver));
    record(Call::MessageBegin {
        destination,
        kind,
        origin,
        receiver,
    });
}

unsafe extern "C" fn message_end() {
    record(Call::MessageEnd);
}

unsafe extern "C" fn write_byte(value: c_int) {
    record(Call::WriteByte(value));
}

unsafe extern "C" fn write_short(value: c_int) {
    record(Call::WriteShort(value));
}

unsafe extern "C" fn write_string(value: *const c_char) {
    record(Call::WriteString(text(value)));
}

unsafe extern "C" fn write_entity(value: c_int) {
    record(Call::WriteEntity(value));
}

macro_rules! fake_log {
    ($name:ident, $function:literal) => {
        unsafe extern "C" fn $name(_plugin: plid_t, format: *const c_char, mut arguments: ...) {
            // SAFETY: the plugin passes one C string after its `%s` format.
            let argument = unsafe { arguments.next_arg::<*const c_char>() };
            record(Call::Log {
                function: $function,
                format:   text(format),
                text:     text(argument),
            });
        }
    };
}

fake_log!(log_console, "LogConsole");
fake_log!(log_message, "LogMessage");
fake_log!(log_error, "LogError");
fake_log!(log_developer, "LogDeveloper");

unsafe extern "C" fn user_message_id(
    _plugin: plid_t,
    name: *const c_char,
    size: *mut c_int,
) -> c_int {
    if text(name) != "SayText" {
        return 0;
    }
    // SAFETY: the plugin passes a writable size.
    unsafe { size.write(-1) };
    SAY_TEXT
}

fn engine_functions() -> enginefuncs_t {
    enginefuncs_t {
        pfnServerPrint: Some(server_print),
        pfnAlertMessage: Some(alert),
        pfnClientPrintf: Some(client_print),
        pfnServerCommand: Some(server_command),
        pfnClientCommand: Some(client_command),
        pfnPrecacheModel: Some(precache_model),
        pfnSzFromIndex: Some(string_from_index),
        pfnAllocString: Some(alloc_string),
        pfnPEntityOfEntIndex: Some(entity_of_index),
        pfnIndexOfEdict: Some(index_of_edict),
        pfnEntOffsetOfPEntity: Some(entity_offset),
        pfnCVarRegister: Some(cvar_register),
        pfnCVarGetPointer: Some(cvar_pointer),
        pfnCVarSetFloat: Some(cvar_set_float),
        pfnCVarSetString: Some(cvar_set_string),
        pfnAddServerCommand: Some(add_server_command),
        pfnCmd_Argc: Some(argument_count),
        pfnCmd_Argv: Some(argument),
        pfnCmd_Args: Some(arguments),
        pfnMessageBegin: Some(message_begin),
        pfnMessageEnd: Some(message_end),
        pfnWriteByte: Some(write_byte),
        pfnWriteShort: Some(write_short),
        pfnWriteString: Some(write_string),
        pfnWriteEntity: Some(write_entity),
        ..Default::default()
    }
}

fn util_functions() -> mutil_funcs_t {
    mutil_funcs_t {
        pfnLogConsole: Some(log_console),
        pfnLogMessage: Some(log_message),
        pfnLogError: Some(log_error),
        pfnLogDeveloper: Some(log_developer),
        pfnGetUserMsgID: Some(user_message_id),
        ..Default::default()
    }
}

/// A plugin loaded into the fake host, the way metamod loads it.
pub struct Loaded {
    pub info:           *mut plugin_info_t,
    pub tables:         META_FUNCTIONS,
    pub meta_globals:   &'static mut meta_globals_t,
    /// The game DLL tables that metamod lent the plugin.
    pub game_functions: gamedll_funcs_t,
}

/// Sets up the edicts and globals that `GiveFnptrsToDll` hands out, with
/// `hl_c01_a1` running. Each call makes a new edict array, as a new map
/// can.
pub fn boot() -> (enginefuncs_t, *mut globalvars_t) {
    // The plugin keeps its state in statics, and the first test thread owns
    // them; plain `cargo test` runs every test in one process.
    assert!(
        std::env::var_os("NEXTEST").is_some(),
        "run these tests with cargo nextest: each test needs its own process"
    );
    let edicts = Box::leak(vec![edict_t::default(); EDICT_COUNT].into_boxed_slice());
    edicts[CONNECTED_PLAYER as usize].v.flags = FL_CLIENT as c_int;
    edicts[FREE_SLOT as usize].free = 1;
    EDICTS.set(edicts.as_mut_ptr());
    let globals = Box::leak(Box::new(globalvars_t {
        maxClients: MAX_CLIENTS,
        maxEntities: EDICT_COUNT as c_int,
        mapname: MAP_NAME,
        pStringBase: STRINGS.as_ptr().cast(),
        ..Default::default()
    }));
    (engine_functions(), globals)
}

/// Runs the entry points in metamod's order: `GiveFnptrsToDll`,
/// `Meta_Query`, `Meta_Attach`. Attaching at `PT_ANYTIME` must succeed.
pub fn load() -> Loaded {
    let (attached, loaded) = try_load(PLUG_LOADTIME::PT_ANYTIME);
    assert_eq!(attached, 1, "Meta_Attach");
    loaded
}

/// Like [`load`], but returns what `Meta_Attach` returned for `now`.
pub fn try_load(now: PLUG_LOADTIME) -> (c_int, Loaded) {
    let (mut functions, globals) = boot();
    let mut util = util_functions();
    let mut info = ptr::null_mut();
    let mut tables = META_FUNCTIONS::default();
    let meta_globals = Box::leak(Box::new(meta_globals_t::default()));
    // metamod-svencoop has one more table, so fill a default value.
    let game_functions = Box::leak(Box::<gamedll_funcs_t>::default());
    game_functions.dllapi_table = Box::leak(Box::<DLL_FUNCTIONS>::default());
    game_functions.newapi_table = Box::leak(Box::<NEW_DLL_FUNCTIONS>::default());
    // SAFETY: every pointer is valid, as metamod would pass it.
    let attached = unsafe {
        crate::GiveFnptrsToDll(&mut functions, globals);
        assert_eq!(
            crate::Meta_Query(c"5:17".as_ptr().cast_mut(), &mut info, &mut util),
            1,
            "Meta_Query"
        );
        crate::Meta_Attach(now, &mut tables, meta_globals, game_functions)
    };
    (
        attached,
        Loaded {
            info,
            tables,
            meta_globals,
            game_functions: *game_functions,
        },
    )
}

/// A getter from `META_FUNCTIONS`.
pub type Getter<T> = Option<unsafe extern "C" fn(*mut T, *mut c_int) -> c_int>;

/// Asks a getter for its table with `version`, as metamod does, and returns
/// the getter's answer, the version it left and the table.
pub fn request<T: Default>(getter: Getter<T>, version: c_int) -> (c_int, c_int, T) {
    let mut table = T::default();
    let mut version = version;
    // SAFETY: both pointers are valid.
    let answer = unsafe { getter.expect("getter")(&mut table, &mut version) };
    (answer, version, table)
}

impl Loaded {
    /// The game DLL hook table, filled in through `GetEntityAPI2`.
    pub fn dll_table(&self) -> DLL_FUNCTIONS {
        let (answer, _, table) = request(self.tables.pfnGetEntityAPI2, INTERFACE_VERSION as c_int);
        assert_eq!(answer, 1, "GetEntityAPI2");
        table
    }

    /// The post hook table, filled in through `GetEntityAPI2_Post`.
    pub fn dll_post_table(&self) -> DLL_FUNCTIONS {
        let (answer, _, table) = request(
            self.tables.pfnGetEntityAPI2_Post,
            INTERFACE_VERSION as c_int,
        );
        assert_eq!(answer, 1, "GetEntityAPI2_Post");
        table
    }

    /// Runs a server command as the engine would after the admin typed it.
    pub fn run_command(&self, words: &[&str]) {
        WORDS.set(
            words
                .iter()
                .map(|word| CString::new(*word).expect("no NUL"))
                .collect(),
        );
        REST.set(CString::new(words[1..].join(" ")).expect("no NUL"));
        let function = COMMANDS.with_borrow(|commands| {
            commands
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(words[0]))
                .map(|(_, function)| *function)
        });
        // SAFETY: the callback the plugin registered.
        unsafe { function.expect("registered command")() };
    }
}
