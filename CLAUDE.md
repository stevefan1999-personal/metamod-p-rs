# metamod-p-rs

Safe Rust API for Metamod plugins (Sven Co-op first). See `README.md` for
usage and `docs/adr/` for decisions.

## Layout

- `src/` — the `metamod_p` crate. `sys` is the raw bindgen output
  (`build.rs`, `wrapper.h`). Safe layer: `engine.rs` (Engine wrappers),
  `entity.rs`, `cvar.rs`, `command.rs`, `message.rs`, `logger.rs`,
  `meta.rs` (MetaResult, Returned), `plugin.rs` (Plugin trait and the
  hook list), `hooks.rs` (the `plugin_trait!` macro, trampolines,
  argument conversion), `export.rs` (C entry points, `export_plugin!`),
  `host.rs` (plugin-wide state), `main_thread.rs` (MainThreadCell).
- `macros/` — `#[metamod_p::plugin]`, a proc macro without dependencies.
- `example-plugin/` — the Sven Co-op example (defaults to i686, `fallguys`).
- `tests/` — fake engine and metamod; drives a plugin through its C
  entry points.

## Commands

- `just build`, `just test` (both header flavours; add
  `--target i686-unknown-linux-gnu` for 32-bit), `just lint`.
- `just smoke [port] [map]` — real Sven Co-op server in `../.svends-rust`
  on port 27016. Never use port 27015 or the shared install directly.
- Tests need `cargo nextest` (one process per test, fresh statics).

## Rules

- Sven Co-op builds use the `fallguys` feature. The `metamod-fallguys`
  submodule stays on the release tag that the smoke test downloads
  (`v20261009a`, interface 5:16): metamod rejects plugins with a newer
  minor interface. Bump both together.
- The two header flavours differ in constness (`char *` against
  `const char *`). Pass `*mut` pointers to C, which fit both, and run
  `just test` and `just lint` for both flavours.
- Add a hook as one line in the `plugin_trait!` list in `src/plugin.rs`;
  raw parameter types must match the bindgen field exactly.
- Never pass text as a C format string: use `"%s"` and pass the text as
  the argument.
- No `static mut`: use `MainThreadCell`. Its `new` needs `T: Send`; crate
  statics of raw pointers to constants use `new_unchecked`. Every `unsafe`
  block gets a `// SAFETY:` comment. Panics must not cross into C
  (`ffi::guard`).
- Metamod copies plugin cvars; read values through the engine's `cvar_t`.
- Formatting follows `.rustfmt.toml` (nightly rustfmt, 4 spaces).
- Conventional commits, one logical change each, no `Co-Authored-By`.
