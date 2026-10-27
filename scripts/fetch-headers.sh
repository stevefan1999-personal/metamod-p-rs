#!/usr/bin/env bash
# The crates bind against two header trees. A full submodule checkout pulls
# hundreds of megabytes that the compiler never reads, so CI fetches only
# the include trees. A developer checkout that already has them is left alone.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ ! -f metamod-p/metamod/meta_api.h ]]; then
  rm -rf metamod-p
  git clone --depth 1 https://github.com/Bots-United/metamod-p.git metamod-p
fi

if [[ ! -f metamod-fallguys/metamod/include/meta_api.h ]]; then
  rev=$(git rev-parse 'HEAD:metamod-fallguys')
  probe=$(mktemp -d)
  git -C "$probe" init -q
  git -C "$probe" fetch --depth 1 https://github.com/hzqst/metamod-fallguys.git "$rev"
  headers=$(git -C "$probe" rev-parse 'FETCH_HEAD:metamod')
  rm -rf "$probe"
  mkdir -p metamod-fallguys
  rm -rf metamod-fallguys/metamod
  git clone --filter=blob:none --sparse https://github.com/metamod-fallguys/metamod.git metamod-fallguys/metamod
  git -C metamod-fallguys/metamod sparse-checkout set include
  git -C metamod-fallguys/metamod fetch --depth 1 origin "$headers"
  git -C metamod-fallguys/metamod checkout --detach FETCH_HEAD
fi

test -f metamod-p/metamod/meta_api.h
test -f metamod-fallguys/metamod/include/HLSDK/dlls/extdll.h
