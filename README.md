# metamod-p-rs

Write [Metamod](https://github.com/Bots-United/metamod-p) plugins for
GoldSrc servers in safe Rust. The main target is Sven Co-op 5.x with
[metamod-svencoop](https://github.com/metamod-svencoop/metamod-svencoop).

| Crate or module | What it holds |
| --- | --- |
| `metamod_p` | The safe API: `Plugin`, `MetaResult`, `PostResult`, `Engine`, `Edict`, cvars, commands, messages, logging. |
| `metamod_p::sys` | The raw bindgen output of the metamod and HLSDK headers. |
| `metamod-p-macros` | The `#[metamod_p::plugin]` attribute (re-exported by `metamod_p`). |
| `example-plugin` | A small plugin for Sven Co-op. |

## Header flavours

The crate builds against one of two header sets:

- default: Metamod-P (`metamod-p` submodule, interface `5:13`), for HLDS.
- feature `fallguys`: metamod-svencoop (`metamod-fallguys` submodule,
  pinned to release `v20261009a`, interface `5:16`), for Sven Co-op.

Use `fallguys` for every Sven Co-op build. The reasons, and why there are
no Metamod-R bindings, are in
[ADR 0001](docs/adr/0001-metamod-flavour.md).

## Requirements

- Rust with the `i686-unknown-linux-gnu` target. `rust-toolchain.toml`
  selects nightly, but only for the rustfmt options in `.rustfmt.toml`.
  The crates build on stable Rust 1.99 or later.
- libclang for bindgen, and 32-bit glibc headers and libraries. On Arch
  Linux: `clang`, `lib32-glibc` and `lib32-gcc-libs` (gcc has multilib).
- [`just`](https://github.com/casey/just) and `cargo-nextest`.

Clone with submodules: `git clone --recursive`.

## Write a plugin

```rust
use std::{error::Error, ffi::CStr};

use metamod_p::{
    CvarFlags, Edict, Engine, LoadTime, MetaResult, Plugin, PluginInfo, RejectReason, StaticCvar,
};

static GREETING: StaticCvar = StaticCvar::new(c"rs_greeting", c"Hello", CvarFlags::SERVER);

struct Hello {
    engine: &'static Engine,
}

#[metamod_p::plugin]
impl Plugin for Hello {
    const INFO: PluginInfo = PluginInfo {
        name:       c"Rust Hello",
        version:    c"0.1.0",
        date:       c"2026-10-27",
        author:     c"you",
        url:        c"https://example.com",
        log_tag:    c"RSHELLO",
        loadable:   LoadTime::AnyTime,
        unloadable: LoadTime::AnyPause,
    };

    fn attach(engine: &'static Engine, now: LoadTime) -> Result<Self, Box<dyn Error>> {
        let greeting = GREETING.register(engine)?;
        engine.register_command(c"rs_hello", move |_args| {
            log::info!("{}", greeting.string())
        });
        log::info!("attached at {now:?}");
        Ok(Self { engine })
    }

    fn client_connect(
        &self,
        player: Edict,
        name: &CStr,
        _address: &CStr,
        reject_reason: &mut RejectReason,
    ) -> MetaResult<bool> {
        if name.to_bytes().is_empty() {
            reject_reason.set("Choose a name first");
            return MetaResult::Supercede(false);
        }
        let map = self.engine.map_name().unwrap_or_default();
        log::info!("{name:?} joins {map} in slot {}", player.index());
        MetaResult::Ignored
    }
}
```

The crate type must be `cdylib`. The full example is in
[`example-plugin`](example-plugin/src/lib.rs).

### Rules

- `#[metamod_p::plugin]` exports `GiveFnptrsToDll`, `Meta_Query`,
  `Meta_Attach` and `Meta_Detach`. It installs only the hook methods that
  the impl overrides, so metamod never calls the plugin for other
  functions. Without the attribute, call `metamod_p::export_plugin!` and
  every hook is installed (`Overrides::All`).
- Hooks take `&self`: the engine calls back into plugins while a hook
  runs. Keep mutable state in `Cell` or `RefCell` fields.
- Each hook has a `_post` twin. Post hooks of functions with a return value
  also get `Returned { original, overridden }`.
- Pre hooks return `MetaResult`: `Supercede(value)` skips the real
  function and returns `value`. Post hooks return `PostResult`:
  `Override(value)` returns `value` instead of the real result, unless a
  pre hook superceded the call. Metamod keeps no value from an override in
  a pre hook, so `MetaResult` has no `Override`.
- Everything runs on the engine thread. The API refuses other threads, and
  `log` lines from other threads go to stderr.
- A panic in a hook is caught at the C boundary and logged as an error;
  the hook then counts as `Ignored`.
- The plugin refuses to detach while one of its hooks or commands runs,
  because metamod closes the library after a detach. Metamod then keeps
  it loaded, except for `meta force_unload`.
- Hooks are skipped (`Ignored`) when the engine passes a null or free
  entity, or a null string, where the signature cannot hold it.
- The engine writes one message at a time. `Engine::message` refuses a
  nested message, but it cannot see messages of the game or other plugins:
  do not start a message in a hook that runs between their
  `MESSAGE_BEGIN` and `MESSAGE_END`, such as `write_byte`.
- `Edict` methods that read or write the entity, or pass it to the engine,
  panic when `is_valid` is false: the slot was freed, or the handle points
  into the edict array of an earlier map. `is_valid` cannot see a new
  entity that reuses a slot with the same serial number, so drop stored
  handles when the map changes.

### API overview

| Task | API |
| --- | --- |
| Print | `Engine::server_print`, `alert`, `client_print` with `PrintKind::{Console, Center, Chat}` |
| Commands | `Engine::server_command`, `server_execute` (`unsafe`: must not unload the plugin), `client_command` (stufftext) |
| Console commands | `Engine::register_command(name, closure)`, `Engine::command_args` |
| Precache | `Engine::precache_model`, `precache_sound`, `precache_generic` (the path is copied into the engine's string pool for the map; use `/` in paths) |
| Cvars | `StaticCvar::new` and `register`, `Engine::cvar`, `Cvar::value`, `string`, `set_value`, `set_string` |
| Messages | `Engine::user_message(c"SayText")`, `Engine::message(destination, kind, \|writer\| ...)` sends `MESSAGE_BEGIN`, the typed writes of the closure and `MESSAGE_END` |
| Entities | `Engine::edict(index)`, `Engine::players()`, `Edict::index`, `is_valid`, and entvars accessors such as `health`/`set_health` |
| Globals | `Engine::map_name`, `time`, `max_clients`, `string(string_t)` (`unsafe`: engine strings only) |
| Logging | the `log` crate: errors go to mutil `LogError`, warnings and info to `LogMessage`, debug and trace to `LogDeveloper` |
| Anything else | `Engine::functions`, `util_functions`, `game_functions` (calls the game DLL and skips all plugins, as `MDLL_*` does), `globals` and `metamod_p::sys` (`unsafe`) |

Text is always passed to C `printf`-style functions as a `%s` argument, so
a `%` in player input cannot be read as a format.

## Build for Sven Co-op

```sh
just build            # target/i686-unknown-linux-gnu/release/libexample_plugin.so
```

Inside `example-plugin/`, plain `cargo build` also targets i686 (see
`.cargo/config.toml`). For Windows, pass `--target i686-pc-windows-msvc` on
a Windows host.

To install the plugin on a server with metamod-svencoop:

1. Copy the library to `svencoop/addons/rust_hello/rust_hello.so`.
2. Add `linux addons/rust_hello/rust_hello.so` to
   `svencoop/addons/metamod/plugins.ini`.
3. Start the server with `-dll addons/metamod/dlls/metamod.so`.

## Test

```sh
just test             # unit, fake-host and doc tests, both header flavours
just test --target i686-unknown-linux-gnu
just lint             # rustfmt and clippy, warnings are errors
```

The integration tests in `tests/` load a test plugin through its real C
entry points into a fake engine and metamod. Use `cargo nextest`: every
test needs its own process, because the plugin keeps its state in statics.

## Smoke test on a real server

```sh
just smoke            # port 27016, map hl_c01_a1
just smoke 27016 hl_c01_a2
just smoke-clean      # remove the isolated server root
```

The smoke test:

1. Builds the example plugin for i686.
2. Makes an isolated server root, `../.svends-rust` by default. It is a
   symlink farm of the dedicated server: the files the server writes
   (`svencoop_addon`, `svencoop/logs`, `svencoop/addons`, the Steam app id,
   the sound cache and node graphs) are real files in the farm.
3. Downloads metamod-svencoop `v20261009a`, checks its SHA-256 and installs
   it with only the example plugin in `plugins.ini`.
4. Starts `svends_i686` with a console fifo, runs `meta list`, `meta cmds`,
   `meta cvars` and `rs_hello`, unloads and loads the plugin again
   (`meta unload`, `meta load`), runs `rs_hello` once more and quits.
5. Checks that the plugin attached and runs, that the ServerActivate hook
   and the command logged their lines, that metamod credits the command
   and cvar to the plugin, that the plugin attaches again after the reload,
   and that no shared file changed.

Set `SVEN_SERVER_DIR` or `STEAM_COMMON` when the dedicated server is not in
the default Steam library, and `SMOKE_ROOT` to move the isolated root.

## License

MIT, see [LICENSE](LICENSE). The metamod and HLSDK headers in the
submodules have their own licenses.
