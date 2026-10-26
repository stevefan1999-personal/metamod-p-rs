# ADR 0001: Metamod flavour for Sven Co-op

- Status: accepted
- Date: 2026-10-27

## Context

Counter-Life runs on Sven Co-op 5.26 (engine build 10269), Linux, 32-bit
(`svends_i686`, `svencoop/dlls/server.so`). The request named "metamod-r".
Three Metamod flavours exist:

- Metamod-R (ReHLDS project), a fork of Metamod-P.
- Metamod-P (Bots-United), the `metamod-p` submodule, interface `5:13`.
- metamod-svencoop (formerly metamod-fallguys), the `metamod-fallguys`
  submodule. Its newest release, `v20261009a`, has interface `5:16`; the
  unreleased `main` branch has `5:17`.

## Evidence

1. metamod-svencoop says it is "a modified version of metamod ... mainly to
   provide server-side feature expansions for Sven Co-op", based on
   Metamod-P (`metamod-fallguys/README.md`). Its game table knows the Sven
   game DLL: `{"svencoop", "server.so", "server.dll", "Sven Coop [Steam
   Version]"}` (`metamod-fallguys/metamod/src/games.h:122`). Its `mutil`
   table adds Sven-specific functions
   (`metamod-fallguys/metamod/include/meta_api.h`).
2. No Metamod-R source is in this workspace. Its upstream README says it "is
   recommended to be run with ReHLDS (API 3.1+)". Its own game table marks
   both `svencoop` entries with "Metamod-r have problems with mod!"
   (`metamod/src/game_support.cpp` in rehlds/Metamod-R). Sven Co-op ships its
   own engine (`engine_i686.so`), so ReHLDS cannot replace it.
3. The Metamod-P v1.21p109 HLSDK ends `enginefuncs_s` with
   `pfnPEntityOfEntIndexAllEntities` (from the Half-Life 25th anniversary
   SDK), but it keeps interface `5:13`. The metamod-svencoop HLSDK does not
   have the field. On i686 the table is 636 bytes with Metamod-P headers and
   632 bytes with metamod-svencoop headers.
   - The copy in `GiveFnptrsToDll` stays in bounds with both header sets.
     Metamod is built with `__METAMOD_BUILD__`
     (`metamod-fallguys/metamod/CMakeLists.txt:53`), so the table that it
     gives to plugins (`g_plugin_engfuncs`, `src/metamod.cpp:79` and
     `:274`) also holds `extra_functions[16]`
     (`include/HLSDK/engine/eiface.h:317-320`).
   - The mismatch is in the hook tables. Metamod allocates the table for
     a plugin's `GetEngineFunctions` and `GetEngineFunctions_Post` with
     its own headers, as `sizeof(enginefuncs_t) - sizeof(extra_functions)`
     (`src/mplugin.cpp:1051-1061`): 632 bytes on Sven Co-op. Metamod accepts
     a `5:13` plugin, because an older minor interface is compatible
     (`src/mplugin.cpp:822-825`). A plugin with Metamod-P headers that uses
     the buffer as a 636-byte `enginefuncs_t` goes past its end.
4. Metamod refuses plugins with a newer minor interface than its own. A
   plugin built from the `main` headers (`5:17`) was rejected by the
   `v20261009a` release on Sven Co-op: `Plugin '<rust_hello.so>' requires
   a newer version of Metamod (Metamod needs at least interface 5:17 not
   the current 5:16)`. Between the tag and `main`, `include/` only gains
   the gamedata functions of `5:17`, which the safe layer does not use.

## Decision

- Sven Co-op plugins use metamod-svencoop: enable the `fallguys` feature.
  The example plugin enables it by default.
- The `metamod-fallguys` submodule is pinned to the release tag that the
  servers run (`v20261009a`, interface `5:16`), not to `main`.
- The crate keeps Metamod-P as its default feature for plain HLDS users.
- Do not add Metamod-R bindings.
- The smoke test pins the metamod-svencoop `v20261009a` release and checks
  its SHA-256 (see `Justfile`).

## Consequences

- Plugins built with `fallguys` report interface `5:16` and load on the
  `v20261009a` release and later; they can use the extended `mutil`
  functions through `metamod_p::sys`.
- To use a newer interface, bump the submodule and `metamod_release` in
  the `Justfile` together, to a release with that interface.
- Metamod calls `Meta_Query` before it compares interface versions, and
  an older metamod has shorter tables than newer headers. So `Meta_Query`
  refuses a metamod with another major or an older minor interface before
  it copies the utility table.
- The safe layer writes only the hook entries that a plugin installs, one
  at a time through the pointer, so it does not go past the end of a
  632-byte hook table. The function slots after `pfnEngCheckParm` still
  differ between the header sets, so use `fallguys` for every Sven Co-op
  build.
- If Sven Co-op changes its engine tables, update the `metamod-fallguys`
  submodule and rebuild.
