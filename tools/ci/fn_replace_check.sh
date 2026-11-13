#!/usr/bin/env bash
# Function replacement end to end (fn-replace): two native mods' `[[native.mixin]]` rows built by the
# launcher's normal native build against a PREBUILT game, with no game rebuild, then run on a
# dedicated server.
#   [FNR_MODS="alpha beta"] [FNR_MOD_ROOT=DIR] tools/ci/fn_replace_check.sh LAUNCHER SDK SCRATCH [RESOURCES]
# FNR_WASM_LAYOUT_PACKAGE=valid.hmod FNR_WASM_STD=std.wasm adds a wasm point-layout refusal check
# to the with-RESOURCES server run (not the build/link-only CI mode):
# its exported layout fingerprint is changed to zero, and its integrity hash is updated.
# SDK is a folder from `tools/prebuilt_game.py build --server` at this checkout's HEAD. With
# RESOURCES (a prepared `horizon-client/26.3` folder), the server starts, takes `/tick` commands
# and stops; every mixin helper prints a line starting `fnr:`.
set -euo pipefail
launcher=$(realpath "$1")
sdk=$(realpath "$2")
scratch=$3
resources=${4:-}
if [ -n "$resources" ]; then resources=$(realpath "$resources"); fi
side=${FNR_SIDE:-server}
case "$side" in server|client) ;; *) echo "FNR_SIDE must be server or client" >&2; exit 2;; esac
root=$(git rev-parse --show-toplevel)
# The game revision is the SDK's: its source is what a patched function is copied out of.
revision=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['commit'])" "$2/horizon-prebuilt.json")
mixins=$(sed -n 's/^pub const REV: &str = "\(.*\)";$/\1/p' "$root/crates/horizon-mod-package/src/mixins_tool.rs")
archive=$(sed -n 's/^pub const PREBUILT_ARCHIVE: &str = "\(.*\)";$/\1/p' "$root/crates/horizon-mod-package/src/index/native.rs")

# FNR_KEEP=1 keeps the launcher's folder (toolchain, horizon-mixins, source): a warm build.
# FNR_MIXINS_FROM_SOURCE=1 builds horizon-mixins from its pinned source, dropping a kept tool: the
# launcher otherwise downloads the prebuilt one, and its source build has to stay proven.
if [ -n "${FNR_KEEP:-}" ]; then rm -rf "$scratch/mods" "$scratch/engine" "$scratch/run" "$scratch/launcher/native/server-builds" "$scratch/launcher/versions"/native-*; else rm -rf "$scratch"; fi
mkdir -p "$scratch"/{home,launcher,data,config,trash,clipboard,desktop,mods}
scratch=$(realpath "$scratch")
data=$scratch/launcher

# The pinned source a patched function is copied out of, from this checkout.
source=$data/native/source/$revision
rm -rf "$source" "$data/native/prebuilt"
mkdir -p "$source"
git -C "$root" archive "$revision" | tar -x -C "$source"
find "$source" -type l -delete
printf '%s' "$revision" > "$source/.horizon-revision"

# The prebuilt game, where the launcher keeps a downloaded one.
host=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p "$data/native/prebuilt"
sdk_suffix=
if [ "$side" = server ]; then sdk_suffix=-server; fi
cp -a "$sdk" "$data/native/prebuilt/$revision-$archive$host$sdk_suffix"

base=$data/versions/ci
mkdir -p "$base/.local/horizon-client/26.3"
touch "$base/.installed"
echo '{}' > "$base/.local/horizon-client/26.3/horizon-resources.json"
version=$data/versions/native-ci
mkdir -p "$version"
touch "$version/.installed"
# On Windows the launcher picks the build target from the installed game's PE machine; any x64
# executable stands in for it, and the launcher itself is one.
case $launcher in *.exe) cp "$launcher" "$version/horizon-client.exe" ;; esac
printf '{"base": "ci", "horizon": "%s", "target": "%s"}\n' "$revision" "$side" > "$version/native-mods.json"

mods=$scratch/mods/fnr
mkdir -p "$mods"
for mod in ${FNR_MODS:-alpha beta}; do
  cp -r "${FNR_MOD_ROOT:-$root/tools/ci/fn-replace-check}/$mod" "$mods/fnr_$mod"
  sed -i "s/^horizon = \"HEAD\"$/horizon = \"$revision\"/; s/^mixins = \"MIXINS\"$/mixins = \"$mixins\"/" "$mods/fnr_$mod/horizon-mod.toml"
  sed -i "s/rev = \"HEAD\"/rev = \"$revision\"/" "$mods/fnr_$mod/Cargo.toml"
done
# The game library: libhorizon_mod_host.so / .dylib, or horizon_mod_host.dll on Windows.
game_library() { ls "$sdk"/deps/*horizon_mod_host.so "$sdk"/deps/*horizon_mod_host.dylib "$sdk"/deps/*horizon_mod_host.dll 2>/dev/null | head -1; }
game_hash=$(sha256sum "$(game_library)" | cut -d' ' -f1)

env_vars=(HORIZON_HOME="$scratch/home" HORIZON_LAUNCHER_HOME="$data" XDG_DATA_HOME="$scratch/data"
  XDG_CONFIG_HOME="$scratch/config" HORIZON_LAUNCHER_NO_SELF_UPDATE=1 HORIZON_NO_AUDIO=1
  HORIZON_LAUNCHER_TRASH="$scratch/trash" HORIZON_LAUNCHER_CLIPBOARD="$scratch/clipboard"
  HORIZON_LAUNCHER_DESKTOP="$scratch/desktop")
started=$(date +%s.%N)
output_args=()
if [ -n "${FNR_MIXINS_FROM_SOURCE:-}" ]; then
  rm -rf "$data/native/tools"
  output_args+=(--mixins-from-source)
fi
if [ "$side" = server ]; then output_args+=(--server-output "$scratch/engine"); fi
env "${env_vars[@]}" "$launcher" native-build "$mods" "${output_args[@]}" --version native-ci --profile optimized \
  --trust-native-code > "$scratch/native-build.out" 2> "$scratch/native-build.err" || built=$?
cat "$scratch/native-build.out"
cat "$scratch/native-build.err" >&2
test -z "${built:-}" || exit "$built"
# The launcher's plan for this build, an instance's or a server folder's export alike: the prebuilt game.
grep -F 'Prebuilt game: only the mods compile' "$scratch/native-build.err"
if [ "$side" = client ]; then
  binary=$(find "$data/versions" -type f -name horizon-client | head -1)
  test -n "$binary"
  cp -a "$(dirname "$binary")" "$scratch/engine"
  cp "$scratch/engine/horizon-client" "$scratch/engine/horizon-engine"
fi
echo "native build: $(python3 -c "print(round($(date +%s.%N) - $started, 1))") s"
test -x "$scratch/engine/horizon-engine" || test -x "$scratch/engine/horizon-engine.exe"
if [ "$side" = server ]; then
  # The server folder's export: the launcher says where the engine is, and installs no instance build.
  grep -F "Built the server engine: $scratch/engine/horizon-engine" "$scratch/native-build.out"
  test -z "$(find "$data/versions" -type f -name 'horizon-client*' ! -path "$version/*")"
fi
log=$data/native/build.log
# Only a `[[native.*]]` row patches a function; a mod of hooks alone must not say it did.
if grep -qs '^\[\[native\.' "$mods"/*/horizon-mod.toml; then
  grep -F 'Function replacement:' "$log"
elif grep -sF 'Function replacement:' "$log"; then
  echo "a set with no [[native.*]] row logged a function replacement"; exit 1
fi
# The prebuilt route, and no game source compiled.
grep -F '"route": "prebuilt"' "$data"/versions/*/native-mods.json > /dev/null || grep -rF 'prebuilt' "$log" > /dev/null
test ! -d "$data/native/stage/crates/horizon-client" || { echo "the game was staged for a source build"; exit 1; }
test "$(sha256sum "$(game_library)" | cut -d' ' -f1)" = "$game_hash"

if [ -n "$resources" ]; then
  run=$scratch/run
  mkdir -p "$run"
  echo eula=true > "$run/eula.txt"
  layout_mod=
  if [ -n "${FNR_WASM_LAYOUT_PACKAGE:-}" ]; then
    cp "${FNR_WASM_STD:?set FNR_WASM_STD to the matching horizon-wasm-std.wasm}" "$scratch/engine/horizon-wasm-std.wasm"
    layout_mod=$(python3 "$root/tools/ci/mismatch_wasm_layout.py" "$FNR_WASM_LAYOUT_PACKAGE" "$run/horizon-mods/layout-refused.hmod")
    printf 'mod.%s:true\n' "$layout_mod" > "$run/horizon-mods.txt"
  fi
  # A loop mod gets a forced chunk of hoppers handing stone along each row, so the loop visits them,
  # and a row of each loop mod's counters beside them.
  hoppers=
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop"* ]]; then
    hoppers='forceload add 0 0
fill 0 100 0 9 100 9 minecraft:hopper[facing=east]{Items:[{Slot:0b,id:"minecraft:stone",count:64}]}'
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop "* ]]; then
    hoppers+=$'\nfill 0 100 11 9 100 11 fnr_loop:counter'
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop_second "* ]]; then
    hoppers+=$'\nfill 0 100 12 9 100 12 fnr_loop_second:counter'
  fi
  (sleep 30; echo "$hoppers"; sleep 5; echo "tick rate 40"; sleep 3; echo "tick rate 13"; sleep 3; echo "tick query"; sleep 3; echo "stop"; sleep 20) |
    (cd "$run" && env "${env_vars[@]}" timeout 180 "$scratch/engine/horizon-engine" --resources "$(realpath "$resources")" \
      --dedicated --nogui) > "$scratch/server.log" 2>&1 || true
  grep -F 'fnr:' "$scratch/server.log"
  if [ -n "$layout_mod" ]; then
    grep -F "wasm mod $layout_mod refused: built for another version of the mod points API (mod layout 0, game layout " "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" alpha "* ]]; then
    grep -F 'OVERWRITE by fnr_alpha' "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop "* ]]; then
    slots=2
    if [[ " ${FNR_MODS:-alpha beta} " == *" loop_second "* ]]; then
      slots=3
      # Two mods' block entities in one world: the second mod's slot counted its own too.
      grep -F 'fnr: loop_second own #1: fnr_loop_second:counter' "$scratch/server.log"
    fi
    # Each slot checks every block entity it took for its own, and never took another mod's.
    if grep -F 'own FOREIGN at' "$scratch/server.log"; then
      echo "a loop slot took another mod's block entity for its own"; exit 1
    fi
    # Both slots ran in priority order, the count picked out its mod's counters, the throttle wrote
    # cooldowns, and the entry's HEAD composed.
    grep -F 'fnr: loop count before #1:' "$scratch/server.log"
    grep -F 'fnr: loop count after #1:' "$scratch/server.log"
    grep -F 'fnr: loop count own #1: fnr_loop:counter' "$scratch/server.log"
    grep -F 'fnr: loop throttle skips #1:' "$scratch/server.log"
    grep -F 'fnr: loop HEAD tick_block_entities' "$scratch/server.log"
    # A call-site row inside the loop body ran in the loop's copy, and an API listener at the entry
    # fired through the same cell as the loop.
    grep -F 'fnr: loop MODIFY_EXPRESSION_VALUE chunk check inside the loop' "$scratch/server.log"
    grep -F 'fnr: loop api head listener at tick_block_entities' "$scratch/server.log"
    grep -F "Published loops: 1 filled by $slots mod slots" "$log"
    # The slot ran the loop again from inside it: the nested run was the game's own, logged once.
    grep -F 'fnr: loop count ran the loop again inside it: true' "$scratch/server.log"
    test "$(grep -cF 'Published loop block_entities.tick: it was entered again while it ran' "$scratch/server.log")" = 1
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop_panic "* ]]; then
    # The slot's panic is the server's crash, and its report names the mod and the loop.
    grep -F 'in native mod fnr_loop_panic, hook loop block_entities.tick, part fnr_loop_panic::Tenth' "$run"/crash-reports/*.txt
    grep -F 'Saving chunks for level' "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" fdshape "* ]]; then
    grep -F 'fnr: native_server_started dedicated=true' "$scratch/server.log"
  fi
fi
