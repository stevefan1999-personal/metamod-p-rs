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
  submodule, interface `5:17`.

## Evidence

1. metamod-svencoop says it is "a modified version of metamod ... mainly to
   provide server-side feature expansions for Sven Co-op", based on
   Metamod-P (`metamod-fallguys/README.md`). Its game table knows the Sven
   game DLL: `{"svencoop", "server.so", "server.dll", "Sven Coop [Steam
   Version]"}` (`metamod-fallguys/metamod/src/games.h:122`). Its `mutil`
   table adds Sven-specific functions up to interface `5:17`
   (`metamod-fallguys/metamod/include/meta_api.h`).
2. No Metamod-R source is in this workspace. Its upstream README says it "is
   recommended to be run with ReHLDS (API 3.1+)". Its own game table marks
   both `svencoop` entries with "Metamod-r have problems with mod!"
   (`metamod/src/game_support.cpp` in rehlds/Metamod-R). Sven Co-op ships its
   own engine (`engine_i686.so`), so ReHLDS cannot replace it.
3. The Metamod-P v1.21p109 HLSDK ends `enginefuncs_s` with
   `pfnPEntityOfEntIndexAllEntities` (from the Half-Life 25th anniversary
   SDK). The metamod-svencoop HLSDK does not have it. On i686 the table is
   636 bytes with Metamod-P headers and 632 bytes with metamod-svencoop
   headers. A plugin that copies the engine table with the wrong headers
   reads past the end of the table on Sven Co-op.

## Decision

- Sven Co-op plugins use metamod-svencoop: enable the `fallguys` feature.
  The example plugin enables it by default.
- The crate keeps Metamod-P as its default feature for plain HLDS users.
- Do not add Metamod-R bindings.
- The smoke test pins the metamod-svencoop `v20261009a` release and checks
  its SHA-256 (see `Justfile`).

## Consequences

- Plugins built with `fallguys` report interface `5:17` and can use the
  extended `mutil` functions through `metamod_p::sys`.
- The safe layer writes only the hook entries that a plugin installs, but
  `GiveFnptrsToDll` copies the whole engine table, so the headers must match
  the engine. Use `fallguys` for every Sven Co-op build.
- If Sven Co-op changes its engine tables, update the `metamod-fallguys`
  submodule and rebuild.
