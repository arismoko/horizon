#!/usr/bin/env bash
# nomod.sh SDK OUT: the game executable with no mods, linked against a server SDK folder from
# `tools/prebuilt_game.py build --server` (the points-tick kit's build.sh link, against the SDK's deps):
# OUT/horizon-client beside the SDK's game libraries and std.
set -euo pipefail
sdk=$(realpath "$1"); out=$2
rm -rf "$out"; mkdir -p "$out"
deps=$sdk/deps
sed -e 's#/\* BUILT_MODS \*/##' -e 's#/\* MOD_ENTRIES \*/##' -e 's#/\* MOD_SLOTS \*/##' -e 's#/\* MOD_REDIRECTS \*/##' \
  -e '/^const _: () = assert!($/,/^);$/d' "$sdk/native_main.rs.template" > "$out/main.rs"
ext() { local f; f=$(ls "$deps"/lib$1.so 2>/dev/null || ls "$deps"/lib$1-*.rmeta | head -1); echo "--extern $1=$f"; }
rustc --edition 2024 -C prefer-dynamic -C opt-level=1 -L "dependency=$deps" -L "dependency=$sdk/host" \
  $(ext horizon_client) $(ext mod_api) $(ext horizon_mod_package) $(ext horizon_mod_host) \
  -C 'link-arg=-Wl,-rpath,$ORIGIN' -o "$out/horizon-client" "$out/main.rs"
cp "$deps"/*.so "$out"/
cp "$(rustc --print sysroot)"/lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd-*.so "$out"/
ls "$out"
