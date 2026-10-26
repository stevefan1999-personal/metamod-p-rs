//! Drives a plugin through its exported C entry points against a fake
//! engine and metamod. Run with `cargo nextest run`: every test gets its own
//! process, so the plugin's statics start fresh each time.

mod support;

use std::{
    cell::{Cell, RefCell},
    error::Error,
    ffi::{c_char, c_int, CStr},
    panic, ptr,
};

use metamod_p::{
    sys::{self, META_RES, PLUG_LOADTIME, PL_UNLOAD_REASON},
    AlertKind, CvarFlags, Edict, Engine, LoadTime, MessageDestination, MessageError, MessageType,
    MetaResult, Plugin, PluginInfo, PrintKind, RejectReason, Returned, StaticCvar, UnloadReason,
};
use support::{
    calls, clear_calls, edict, load, Call, CONNECTED_PLAYER, EMPTY_PLAYER_SLOT, FREE_SLOT,
    PRECACHED_MODEL_INDEX, SAY_TEXT,
};

thread_local! {
    static ENGINE: Cell<Option<&'static Engine>> = const { Cell::new(None) };
    static EVENTS: RefCell<Vec<String>> = RefCell::default();
}

fn event(text: String) {
    EVENTS.with_borrow_mut(|events| events.push(text));
}

fn events() -> Vec<String> {
    EVENTS.with_borrow(Clone::clone)
}

fn engine() -> &'static Engine {
    ENGINE.get().expect("attached")
}

static TEST_CVAR: StaticCvar = StaticCvar::new(c"rs_test", c"7", CvarFlags::SERVER);

struct TestPlugin;

#[metamod_p::plugin]
impl Plugin for TestPlugin {
    const INFO: PluginInfo = PluginInfo {
        name:       c"Test Plugin",
        version:    c"1.0",
        date:       c"2026-10-27",
        author:     c"tests",
        url:        c"https://example.invalid",
        log_tag:    c"TEST",
        loadable:   LoadTime::AnyTime,
        unloadable: LoadTime::AnyPause,
    };

    fn attach(engine: &'static Engine, now: LoadTime) -> Result<Self, Box<dyn Error>> {
        ENGINE.set(Some(engine));
        engine.register_command(c"rs_hello", |args| {
            let words: Vec<_> = args.iter().map(CStr::to_string_lossy).collect();
            event(format!("rs_hello {words:?} rest={:?}", args.rest()));
        });
        event(format!("attach {now:?}"));
        Ok(Self)
    }

    fn detach(self, now: LoadTime, reason: UnloadReason) {
        event(format!("detach {now:?} {reason:?}"));
    }

    fn client_connect(
        &self,
        player: Edict,
        name: &CStr,
        _address: &CStr,
        reject_reason: &mut RejectReason,
    ) -> MetaResult<bool> {
        event(format!("connect {} {name:?}", player.index()));
        match name.to_bytes() {
            b"handled" => MetaResult::Handled,
            b"override" => MetaResult::Override(true),
            b"supercede" => {
                reject_reason.set("Rust says no");
                MetaResult::Supercede(false)
            }
            _ => MetaResult::Ignored,
        }
    }

    fn client_connect_post(
        &self,
        _player: Edict,
        _name: &CStr,
        _address: &CStr,
        _reject_reason: &mut RejectReason,
        returned: Returned<bool>,
    ) -> MetaResult<bool> {
        event(format!("connect_post {returned:?}"));
        MetaResult::Override(!returned.original)
    }

    fn think(&self, _entity: Edict) -> MetaResult<()> {
        panic!("think failed")
    }

    fn server_activate(&self, edict_count: i32, max_clients: i32) -> MetaResult<()> {
        let map = engine().map_name();
        event(format!("activate {map:?} {edict_count} {max_clients}"));
        MetaResult::Handled
    }

    fn precache_model(&self, path: &CStr) -> MetaResult<i32> {
        event(format!("precache {path:?}"));
        MetaResult::Supercede(42)
    }
}

/// Calls the plugin's ClientConnect pre hook like metamod would.
fn connect(loaded: &support::Loaded, player: *mut sys::edict_t, name: &CStr) -> (c_int, String) {
    let hook = loaded
        .dll_table()
        .pfnClientConnect
        .expect("ClientConnect hook");
    let mut reject_reason = [0 as c_char; RejectReason::CAPACITY];
    // SAFETY: valid strings and buffer, as the engine passes them.
    let result = unsafe {
        hook(
            player,
            name.as_ptr(),
            c"127.0.0.1:27005".as_ptr(),
            &mut reject_reason,
        )
    };
    // SAFETY: the buffer is NUL-terminated.
    let reason = unsafe { CStr::from_ptr(reject_reason.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    (result, reason)
}

#[test]
fn query_hands_metamod_the_plugin_info() {
    let loaded = load();
    // SAFETY: Meta_Query pointed `info` at the plugin's static info.
    let info = unsafe { &*loaded.info };
    // SAFETY: the info strings are static C strings.
    unsafe {
        assert_eq!(CStr::from_ptr(info.name), c"Test Plugin");
        assert_eq!(CStr::from_ptr(info.ifvers), sys::META_INTERFACE_VERSION);
        assert_eq!(CStr::from_ptr(info.logtag), c"TEST");
    }
    assert_eq!(info.loadable, PLUG_LOADTIME::PT_ANYTIME);
    assert_eq!(events(), ["attach AnyTime"]);
}

#[test]
fn attach_only_offers_tables_with_overridden_hooks() {
    let loaded = load();
    assert!(loaded.tables.pfnGetEntityAPI2.is_some());
    assert!(loaded.tables.pfnGetEntityAPI2_Post.is_some());
    assert!(loaded.tables.pfnGetNewDLLFunctions.is_none());
    assert!(loaded.tables.pfnGetNewDLLFunctions_Post.is_none());
    assert!(loaded.tables.pfnGetEngineFunctions.is_some());
    assert!(loaded.tables.pfnGetEngineFunctions_Post.is_none());
    assert!(loaded.tables.pfnGetEntityAPI.is_none());
}

#[test]
fn tables_hold_only_the_overridden_hooks() {
    let loaded = load();
    let table = loaded.dll_table();
    assert!(table.pfnClientConnect.is_some());
    assert!(table.pfnServerActivate.is_some());
    assert!(table.pfnThink.is_some());
    assert!(table.pfnSpawn.is_none());
    assert!(table.pfnClientDisconnect.is_none());
    assert!(table.pfnStartFrame.is_none());
    let post = loaded.dll_post_table();
    assert!(post.pfnClientConnect.is_some());
    assert!(post.pfnServerActivate.is_none());
}

#[test]
fn table_request_with_another_interface_version_is_refused() {
    let loaded = load();
    let mut table = sys::DLL_FUNCTIONS::default();
    let mut version = 1;
    let getter = loaded.tables.pfnGetEntityAPI2.expect("getter");
    // SAFETY: valid pointers.
    assert_eq!(unsafe { getter(&mut table, &mut version) }, 0);
    assert_eq!(version, sys::INTERFACE_VERSION as c_int);
    assert!(table.pfnClientConnect.is_none());
    // SAFETY: metamod may pass null; the plugin must refuse, not crash.
    assert_eq!(unsafe { getter(ptr::null_mut(), ptr::null_mut()) }, 0);
}

#[test]
fn hook_results_set_metamod_result_and_return_value() {
    let loaded = load();
    let player = edict(CONNECTED_PLAYER);
    let cases: [(&CStr, META_RES, c_int, &str); 4] = [
        (c"ignored", META_RES::MRES_IGNORED, 0, ""),
        (c"handled", META_RES::MRES_HANDLED, 0, ""),
        (c"override", META_RES::MRES_OVERRIDE, 1, ""),
        (c"supercede", META_RES::MRES_SUPERCEDE, 0, "Rust says no"),
    ];
    for (name, expected_result, expected_return, expected_reason) in cases {
        loaded.meta_globals.mres = META_RES::MRES_UNSET;
        let (returned, reason) = connect(&loaded, player, name);
        assert_eq!(loaded.meta_globals.mres, expected_result, "{name:?}");
        assert_eq!(returned, expected_return, "{name:?}");
        assert_eq!(reason, expected_reason, "{name:?}");
    }
}

#[test]
fn post_hook_reads_the_original_and_overridden_values() {
    let loaded = load();
    let mut original: c_int = 1;
    let mut overridden: c_int = 0;
    loaded.meta_globals.orig_ret = ptr::from_mut(&mut original).cast();
    loaded.meta_globals.override_ret = ptr::from_mut(&mut overridden).cast();
    loaded.meta_globals.status = META_RES::MRES_OVERRIDE;
    let hook = loaded.dll_post_table().pfnClientConnect.expect("post hook");
    let mut reject_reason = [0 as c_char; RejectReason::CAPACITY];
    // SAFETY: valid arguments, as the engine passes them.
    let result = unsafe {
        hook(
            edict(CONNECTED_PLAYER),
            c"post".as_ptr(),
            c"127.0.0.1".as_ptr(),
            &mut reject_reason,
        )
    };
    assert_eq!(
        result, 0,
        "the post hook overrides with the negated original"
    );
    assert_eq!(loaded.meta_globals.mres, META_RES::MRES_OVERRIDE);
    let expected = Returned {
        original:   true,
        overridden: Some(false),
    };
    assert_eq!(
        events().last().cloned(),
        Some(format!("connect_post {expected:?}"))
    );

    // Without an override status, `override_ret` may be stale and is not read.
    loaded.meta_globals.status = META_RES::MRES_HANDLED;
    // SAFETY: as above.
    unsafe {
        hook(
            edict(CONNECTED_PLAYER),
            c"post".as_ptr(),
            c"127.0.0.1".as_ptr(),
            &mut reject_reason,
        )
    };
    let expected = Returned {
        original:   true,
        overridden: None,
    };
    assert_eq!(
        events().last().cloned(),
        Some(format!("connect_post {expected:?}"))
    );
}

#[test]
fn null_entity_skips_the_hook() {
    let loaded = load();
    loaded.meta_globals.mres = META_RES::MRES_UNSET;
    let (returned, _) = connect(&loaded, ptr::null_mut(), c"handled");
    assert_eq!(returned, 0);
    assert_eq!(loaded.meta_globals.mres, META_RES::MRES_IGNORED);
    connect(&loaded, edict(FREE_SLOT), c"handled");
    assert_eq!(events(), ["attach AnyTime"], "the hook never ran");
}

#[test]
fn panicking_hook_is_contained_and_logged() {
    let loaded = load();
    clear_calls();
    loaded.meta_globals.mres = META_RES::MRES_UNSET;
    let hook = loaded.dll_table().pfnThink.expect("think hook");
    // SAFETY: a valid entity; the panic must not unwind into this "C" caller.
    unsafe { hook(edict(CONNECTED_PLAYER)) };
    assert_eq!(loaded.meta_globals.mres, META_RES::MRES_IGNORED);
    assert!(
        calls().iter().any(|call| matches!(
            call,
            Call::Log { function: "LogError", format, text } if format == "%s" && text.contains("think failed")
        )),
        "{:?}",
        calls()
    );
}

#[test]
fn engine_hook_supercedes_with_its_value() {
    let loaded = load();
    let mut table = sys::enginefuncs_t::default();
    let mut version = sys::ENGINE_INTERFACE_VERSION as c_int;
    let getter = loaded.tables.pfnGetEngineFunctions.expect("engine getter");
    // SAFETY: valid pointers.
    assert_eq!(unsafe { getter(&mut table, &mut version) }, 1);
    assert!(table.pfnPrecacheSound.is_none());
    let hook = table.pfnPrecacheModel.expect("precache hook");
    // SAFETY: a valid path.
    assert_eq!(
        unsafe { hook(c"models/player.mdl".as_ptr().cast_mut()) },
        42
    );
    assert_eq!(loaded.meta_globals.mres, META_RES::MRES_SUPERCEDE);
    assert_eq!(
        events().last().cloned(),
        Some("precache \"models/player.mdl\"".into())
    );
}

#[test]
fn server_activate_sees_the_map_name() {
    let loaded = load();
    let hook = loaded.dll_table().pfnServerActivate.expect("activate hook");
    // SAFETY: the edict list is the edict array.
    unsafe { hook(edict(0), 3, 4) };
    assert_eq!(loaded.meta_globals.mres, META_RES::MRES_HANDLED);
    assert_eq!(
        events().last().cloned(),
        Some("activate Some(\"hl_c01_a1\") 3 4".into())
    );
}

#[test]
fn message_writer_sends_typed_writes_between_begin_and_end() {
    load();
    clear_calls();
    let player = engine().edict(CONNECTED_PLAYER).expect("player");
    let say_text = engine().user_message(c"SayText").expect("SayText");
    assert_eq!(say_text, MessageType(SAY_TEXT));
    assert_eq!(engine().user_message(c"NoSuchMessage"), None);
    engine()
        .message(MessageDestination::One(player), say_text)
        .expect("no open message")
        .byte(1)
        .short(-2)
        .unsigned_short(u16::MAX)
        .string(c"hi")
        .entity(player);
    assert_eq!(
        calls(),
        [
            Call::MessageBegin {
                destination: sys::MSG_ONE as c_int,
                kind:        SAY_TEXT,
                origin:      None,
                receiver:    Some(CONNECTED_PLAYER),
            },
            Call::WriteByte(1),
            Call::WriteShort(-2),
            Call::WriteShort(c_int::from(u16::MAX)),
            Call::WriteString("hi".into()),
            Call::WriteEntity(CONNECTED_PLAYER),
            Call::MessageEnd,
        ]
    );
}

#[test]
fn second_message_is_refused_while_one_is_open() {
    load();
    clear_calls();
    let origin = [1.0, 2.0, 3.0];
    let writer = engine()
        .message(
            MessageDestination::Visible(origin),
            MessageType::TEMP_ENTITY,
        )
        .expect("first message");
    assert_eq!(
        engine()
            .message(MessageDestination::All, MessageType::TEMP_ENTITY)
            .err(),
        Some(MessageError::AlreadyWriting)
    );
    drop(writer);
    assert!(engine()
        .message(MessageDestination::All, MessageType::TEMP_ENTITY)
        .is_ok());
    assert_eq!(
        calls()[0],
        Call::MessageBegin {
            destination: sys::MSG_PVS as c_int,
            kind:        sys::SVC_TEMPENTITY as c_int,
            origin:      Some(origin),
            receiver:    None,
        }
    );
}

#[test]
fn message_end_is_sent_when_a_panic_unwinds_the_writer() {
    load();
    clear_calls();
    let unwound = panic::catch_unwind(|| {
        let mut writer = engine()
            .message(MessageDestination::Broadcast, MessageType::TEMP_ENTITY)
            .expect("message");
        writer.byte(7);
        panic!("mid-message");
    });
    assert!(unwound.is_err());
    assert_eq!(calls().last(), Some(&Call::MessageEnd));
    assert!(engine()
        .message(MessageDestination::All, MessageType::TEMP_ENTITY)
        .is_ok());
}

#[test]
fn static_cvar_registers_and_reads_the_engine_copy() {
    load();
    clear_calls();
    let cvar = TEST_CVAR.register(engine()).expect("registered");
    assert_eq!(
        calls(),
        [Call::CvarRegister {
            name:   "rs_test".into(),
            string: "7".into(),
            flags:  sys::FCVAR_SERVER as c_int,
        }]
    );
    assert_eq!(cvar.name(), c"rs_test");
    assert_eq!((cvar.value(), cvar.string()), (7.0, "7".to_string()));
    cvar.set_value(3.5);
    assert_eq!((cvar.value(), cvar.string()), (3.5, "3.5".to_string()));
    cvar.set_string("12");
    assert_eq!(cvar.value(), 12.0);
    assert_eq!(
        engine().cvar(c"rs_test").map(|found| found.value()),
        Some(12.0)
    );
    assert!(engine().cvar(c"sv_missing").is_none());
}

#[test]
fn console_command_runs_its_closure_by_name_without_case() {
    let loaded = load();
    assert!(calls().contains(&Call::AddServerCommand("rs_hello".into())));
    loaded.run_command(&["RS_HELLO", "big", "world"]);
    assert_eq!(
        events().last().cloned(),
        Some("rs_hello [\"RS_HELLO\", \"big\", \"world\"] rest=\"big world\"".into())
    );
}

#[test]
fn logged_text_is_an_argument_not_a_format_string() {
    load();
    clear_calls();
    log::info!("100% %s %n");
    log::warn!("careful");
    assert_eq!(
        calls(),
        [
            Call::Log {
                function: "LogMessage",
                format:   "%s".into(),
                text:     "100% %s %n".into(),
            },
            Call::Log {
                function: "LogMessage",
                format:   "%s".into(),
                text:     "WARNING: careful".into(),
            },
        ]
    );
}

#[test]
fn lines_logged_before_the_first_map_go_to_the_console() {
    load();
    // SAFETY: the fake globals are plain memory owned by the test.
    unsafe { (*engine().globals().as_ptr()).mapname = 0 };
    clear_calls();
    log::info!("attached");
    log::error!("broken");
    log::debug!("hidden below the default level");
    assert_eq!(
        calls(),
        [
            Call::Log {
                function: "LogConsole",
                format:   "%s".into(),
                text:     "[TEST] attached".into(),
            },
            Call::Log {
                function: "LogConsole",
                format:   "%s".into(),
                text:     "[TEST] ERROR: broken".into(),
            },
        ]
    );
}

#[test]
fn printed_and_stuffed_text_is_never_a_format_string() {
    load();
    clear_calls();
    let player = engine().edict(CONNECTED_PLAYER).expect("player");
    engine().alert(AlertKind::Logged, "50%n");
    engine().client_command(player, "say %s");
    engine().client_print(player, PrintKind::Center, "hi\0there");
    engine().server_command("changelevel hl_c01_a2");
    engine().server_print("up\n");
    assert_eq!(
        calls(),
        [
            Call::Alert {
                kind:   sys::ALERT_TYPE::at_logged.0,
                format: "%s".into(),
                text:   "50%n".into(),
            },
            Call::ClientCommand {
                player: CONNECTED_PLAYER,
                format: "%s".into(),
                text:   "say %s\n".into(),
            },
            Call::ClientPrint {
                player: CONNECTED_PLAYER,
                kind:   sys::PRINT_TYPE::print_center.0,
                text:   "hithere".into(),
            },
            Call::ServerCommand("changelevel hl_c01_a2\n".into()),
            Call::ServerPrint("up\n".into()),
        ]
    );
}

#[test]
fn precache_passes_the_static_path() {
    load();
    clear_calls();
    assert_eq!(
        engine().precache_model(c"models/cs16/v_ak47.mdl"),
        PRECACHED_MODEL_INDEX
    );
    assert_eq!(
        calls(),
        [Call::PrecacheModel("models/cs16/v_ak47.mdl".into())]
    );
}

#[test]
fn entities_are_checked_against_the_engine() {
    load();
    let player = engine().edict(CONNECTED_PLAYER).expect("player slot");
    assert_eq!(player.index(), CONNECTED_PLAYER);
    assert!(player.is_client() && player.is_valid());
    assert!(
        engine().edict(FREE_SLOT).is_none(),
        "free slots have no entity"
    );
    assert!(engine().edict(10_000).is_none(), "out of range");
    let players: Vec<_> = engine().players().map(|player| player.index()).collect();
    assert_eq!(
        players,
        [CONNECTED_PLAYER],
        "slot {EMPTY_PLAYER_SLOT} has no client"
    );

    player.set_health(55.0);
    player.set_origin([1.0, 2.0, 3.0]);
    assert_eq!((player.health(), player.origin()), (55.0, [1.0, 2.0, 3.0]));
    assert_eq!(player.class_name(), None, "null string_t is no string");

    // The engine bumps the serial number when it frees and reuses a slot.
    // SAFETY: the fake edict array is plain memory owned by the test.
    unsafe { (*edict(CONNECTED_PLAYER)).serialnumber += 1 };
    assert!(!player.is_valid());
}

#[test]
fn detach_drops_the_plugin_and_its_commands() {
    let loaded = load();
    // SAFETY: called as metamod would.
    assert_eq!(
        unsafe { Meta_Detach(PLUG_LOADTIME::PT_ANYTIME, PL_UNLOAD_REASON::PNL_COMMAND) },
        1
    );
    assert_eq!(
        events().last().cloned(),
        Some("detach AnyTime Command".into())
    );
    clear_calls();
    loaded.run_command(&["rs_hello"]);
    assert!(calls()
        .iter()
        .any(|call| matches!(call, Call::Log { text, .. } if text.contains("no handler"))));
    loaded.meta_globals.mres = META_RES::MRES_UNSET;
    let (returned, _) = connect(&loaded, edict(CONNECTED_PLAYER), c"handled");
    assert_eq!(
        (returned, loaded.meta_globals.mres),
        (0, META_RES::MRES_IGNORED)
    );
}

#[test]
fn null_pointers_into_the_entry_points_are_refused() {
    // SAFETY: metamod could pass null; the plugin must refuse, not crash.
    unsafe {
        GiveFnptrsToDll(ptr::null_mut(), ptr::null_mut());
        assert_eq!(
            Meta_Query(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()),
            0
        );
        assert_eq!(
            Meta_Attach(
                PLUG_LOADTIME::PT_ANYTIME,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut()
            ),
            0
        );
    }
    let (mut functions, globals) = support::boot();
    let mut info = ptr::null_mut();
    // SAFETY: valid engine tables, but no utility table.
    unsafe {
        GiveFnptrsToDll(&mut functions, globals);
        assert_eq!(
            Meta_Query(c"5:17".as_ptr().cast_mut(), &mut info, ptr::null_mut()),
            0
        );
    }
    assert!(
        !info.is_null(),
        "the info is handed out even when the query fails"
    );
}
