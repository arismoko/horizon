#!/usr/bin/env bash
set -euo pipefail
mode=$1
root=$PWD
cache="$HOME/.cache/horizon-pickup-invalidation"
base="$cache/base-$mode"
stage="$cache/stage-$mode"
target="$cache/target"
rm -rf "$base" "$stage"
mkdir -p "$base"
git archive HEAD | tar -x -C "$base"
if [ -L "$base/CLAUDE.md" ]; then rm "$base/CLAUDE.md"; cp "$base/AGENTS.md" "$base/CLAUDE.md"; fi
git rev-parse HEAD > "$base/.horizon-revision"
cli="$HOME/.cache/horizon-sample-mod/mixins/target/debug/horizon-mixins"
"$cli" apply --workspace "$base" --mods "$root/probe-mods" --out "$stage"
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="$target"
(cd "$stage" && cargo build --offline --profile fast -p horizon-client --bin horizon-client --bin horizon-ctl)
for variant in allow cancel; do
  home=$(mktemp -d "$cache/game-$mode-$variant.XXXXXX")
  export HORIZON_HOME="$home/game" HORIZON_LAUNCHER_HOME="$home/launcher" XDG_DATA_HOME="$home/data" XDG_CONFIG_HOME="$home/config"
  export HORIZON_LAUNCHER_NO_SELF_UPDATE=1 HORIZON_NO_AUDIO=1 HORIZON_LAUNCHER_TRASH="$home/trash" HORIZON_LAUNCHER_CLIPBOARD="$home/clipboard" HORIZON_LAUNCHER_DESKTOP="$home/desktop"
  export PICKUP_PROOF="$home/marker"
  if [ "$variant" = cancel ]; then export PICKUP_CANCEL=1; else unset PICKUP_CANCEL || true; fi
  ctl="$target/fast/horizon-ctl"; sock="$home/ctl.sock"
  "$ctl" --socket "$sock" launch --game "$target/fast/horizon-client" --world Pickup --creative --offscreen --wait --resources "$HOME/.cache/horizon-upstream-build2-input/local/horizon-client/26.3" --game-dir "$home/game"
  "$ctl" --socket "$sock" command 'summon minecraft:item ~ ~ ~ {Item:{id:"minecraft:diamond",count:1},PickupDelay:0s}'
  for _ in $(seq 1 30); do [ -e "$PICKUP_PROOF" ] && break; sleep 1; done
  test -f "$PICKUP_PROOF"; cat "$PICKUP_PROOF"; echo " ($mode, $variant)"
  sleep 3
  if [ "$mode" = before ]; then
    grep -q "index out of bounds" "$home/ctl.log"
    echo "Expected index out of bounds panic observed: $variant"
  else
    if grep -q "panicked at" "$home/ctl.log"; then cat "$home/ctl.log"; exit 1; fi
    "$ctl" --socket "$sock" state
    "$ctl" --socket "$sock" screenshot "$root/pickup-$variant.png"
    echo "Server remained responsive: $variant"
  fi
  "$ctl" --socket "$sock" quit || true
done
