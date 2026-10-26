set shell := ['bash', '-euo', 'pipefail', '-c']

steam_common := env('STEAM_COMMON', home_directory() / '.local/share/Steam/steamapps/common')
server_dir := env('SVEN_SERVER_DIR', steam_common / 'Sven Co-op Dedicated Server')
smoke_root := env('SMOKE_ROOT', parent_directory(justfile_directory()) / '.svends-rust')
plugin_target := env('PLUGIN_TARGET', 'i686-unknown-linux-gnu')

# The metamod-svencoop release that the smoke test runs. Its interface
# version must not be older than the headers in metamod-fallguys.
metamod_release := 'v20261009a'
metamod_archive := 'metamod-fallguys-ubuntu-i386.zip'
metamod_sha256 := '552ae6d1faf184498f29d0c686515f56618eb55229970d7a687d8d5c74ed2d78'
metamod_url := 'https://github.com/metamod-svencoop/metamod-svencoop/releases/download/' + metamod_release + '/' + metamod_archive

default:
  @just --list

# Build the example plugin: just build i686-unknown-linux-gnu release
[group('build')]
build target=plugin_target profile='release':
  cargo build -p example-plugin --target {{target}} --profile {{profile}}

# Run the unit, fake-host and doc tests for both header flavours: just test --target i686-unknown-linux-gnu
[group('check')]
test *args:
  cargo nextest run -p metamod-p {{args}}
  cargo nextest run -p metamod-p --features fallguys {{args}}
  cargo test --doc -p metamod-p
  cargo test --doc -p metamod-p --features fallguys

# Check formatting and run clippy for both header flavours: just lint
[group('check')]
lint:
  cargo fmt --all --check
  cargo clippy -p metamod-p -p metamod-p-macros --all-targets -- -D warnings
  cargo clippy --workspace --all-targets -- -D warnings

# Make an isolated server root with metamod-svencoop and only the example plugin: just smoke-setup
[group('smoke')]
smoke-setup root=smoke_root target=plugin_target profile='release': (build target profile)
  #!/usr/bin/env bash
  set -euo pipefail
  root='{{root}}'
  server='{{server_dir}}'
  test -x "$server/svends_i686" || { echo "no dedicated server in $server" >&2; exit 1; }
  # Game files are shared read-only through symlinks; everything the server
  # writes goes to real directories of the farm.
  test "$(realpath -m "$root")" != "$(realpath -m "$server")" || { echo 'the root must not be the shared install' >&2; exit 1; }
  test -e "$root/svends_i686" || cp -as "$server" "$root"
  for dir in svencoop_addon svencoop/logs svencoop/addons; do
    rm -rf "${root:?}/$dir" && mkdir -p "$root/$dir"
  done
  # Files the server rewrites in place (Steam app id, sound cache, node
  # graphs) become real copies, so the writes stay in the farm.
  for path in steam_appid.txt svencoop/maps/soundcache svencoop/maps/graphs; do
    rm -rf "${root:?}/$path" && cp -a "$server/$path" "$root/$path"
  done
  download="$root/.download"
  archive="$download/{{metamod_archive}}"
  mkdir -p "$download"
  if ! echo '{{metamod_sha256}}  '"$archive" | sha256sum --check --status 2>/dev/null; then
    curl -fsSL -o "$archive" '{{metamod_url}}'
    echo '{{metamod_sha256}}  '"$archive" | sha256sum --check
  fi
  rm -rf "$download/unpacked" && unzip -q "$archive" -d "$download/unpacked"
  cp -r "$download/unpacked/build/addons/." "$root/svencoop/addons/"
  install -D -m 755 '{{justfile_directory()}}/target/{{target}}/{{ if profile == "dev" { "debug" } else { profile } }}/libexample_plugin.so' \
    "$root/svencoop/addons/rust_hello/rust_hello.so"
  # Only the example plugin; the plugins that ship with the release stay off.
  printf 'linux addons/rust_hello/rust_hello.so\n' > "$root/svencoop/addons/metamod/plugins.ini"
  echo "isolated server root ready: $root"

# Boot the isolated server, run `meta list`, `rs_hello` and a plugin reload, check the output, stop: just smoke 27016 hl_c01_a1
[group('smoke')]
smoke port='27016' map='hl_c01_a1' root=smoke_root target=plugin_target: (smoke-setup root target)
  #!/usr/bin/env bash
  set -euo pipefail
  root='{{root}}'
  state="$root/.smoke-{{port}}"
  rm -rf "$state" && mkdir -p "$state" && mkfifo "$state/console"
  touch "$state/started"
  cd "$root"
  # metamod-svencoop appends its warnings and the plugin log lines to
  # metamod.log in the working directory, also before the first map, when the
  # console drops them. Start it fresh, so the checks see only this run; rm
  # instead of truncating, because a symlink would lead into the shared install.
  rm -f metamod.log && touch metamod.log
  export LD_LIBRARY_PATH="$PWD:$PWD/redist${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  # The holder keeps the pipe open, so the server never reads EOF between commands.
  sleep infinity > "$state/console" &
  holder=$!
  ./svends_i686 -dll addons/metamod/dlls/metamod.so -console -port {{port}} +sv_lan 1 +map {{map}} \
    < "$state/console" > "$state/server.log" 2>&1 &
  server=$!
  trap 'kill "$server" "$holder" 2>/dev/null || true' EXIT
  for _ in $(seq 1 90); do
    rg -q 'Started map' "$state/server.log" && break
    kill -0 "$server" 2>/dev/null || { tail -n 60 "$state/server.log"; echo 'the server exited early' >&2; exit 1; }
    sleep 1
  done
  rg -q 'Started map' "$state/server.log" || { tail -n 60 "$state/server.log"; echo 'no map after 90 s' >&2; exit 1; }
  printf 'meta list\nmeta cmds\nmeta cvars\nrs_hello from the smoke test\n' > "$state/console"
  # Unloading and loading again runs detach, dlclose and a fresh attach.
  printf 'meta unload rust_hello\nmeta load addons/rust_hello/rust_hello.so\nrs_hello after reload\n' > "$state/console"
  sleep 3
  printf 'quit\n' > "$state/console"
  for _ in $(seq 1 20); do kill -0 "$server" 2>/dev/null || break; sleep 1; done
  log="$state/server.log"
  echo "--- plugin lines in $log"
  rg -n -i 'rshello|rust hello|rs_hello|rust_hello' "$log" || true
  failed=0
  check() { rg -q "$2" "${3:-$log}" && echo "ok: $1" || { echo "FAILED: $1" >&2; failed=1; }; }
  # Metamod only warns about a broken plugin and keeps it, and the plugin
  # logs a contained panic as an error, so the positive checks can miss both.
  refuse() { if rg -n "$2" "$log" metamod.log; then echo "FAILED: $1" >&2; failed=1; else echo "ok: $1"; fi; }
  check 'the plugin attached at startup' '\[RSHELLO\] attached at Startup'
  check 'meta list shows the plugin running' 'Rust Hello.* RUN '
  check 'the ServerActivate hook logged the map' '\[RSHELLO\] map {{map}} is active'
  # The game precaches before the server log opens, so the console drops the
  # line; metamod.log has it, without the log tag.
  check 'a post hook on an engine function read the precache result' \
    '^first precache: ".+" got Returned \{ original: [0-9]+, overridden: None \}' metamod.log
  check 'rs_hello ran its closure and read the cvar' '\[RSHELLO\] rs_hello: Hello from Rust \(arguments: "from the smoke test"\)'
  check 'metamod credits rs_hello to the plugin' 'Rust Hello +rs_hello'
  check 'metamod credits rs_greeting to the plugin' 'Rust Hello +rs_greeting +[0-9.]+ +Hello from Rust'
  check 'the plugin attached again after meta unload and meta load' '\[RSHELLO\] attached at AnyTime'
  check 'rs_hello works after the reload' 'rs_hello: Hello from Rust \(arguments: "after reload"\)'
  check 'the NEW_DLL GameShutdown hook ran at quit' '\[RSHELLO\] the game shuts down'
  refuse 'metamod reported no broken plugin and no hook panicked' \
    "didn't set meta_result|Failure calling|Failed to attach|not valid in Post|panicked"
  # The farm only shares files through symlinks, so none of those may change.
  changed=$(find -L "$root" -newer "$state/started" -type f -print0 \
    | while IFS= read -r -d '' file; do if test -L "$file"; then echo "$file"; fi; done)
  if test -n "$changed"; then echo "FAILED: shared files changed during the run:" >&2; echo "$changed" >&2; failed=1; else echo 'ok: no shared file changed'; fi
  exit "$failed"

# Remove the isolated server root: just smoke-clean
[group('smoke')]
smoke-clean root=smoke_root:
  #!/usr/bin/env bash
  set -euo pipefail
  test "$(realpath -m '{{root}}')" != "$(realpath -m '{{server_dir}}')" || { echo 'refusing to remove the shared install' >&2; exit 1; }
  rm -rf '{{root}}'
