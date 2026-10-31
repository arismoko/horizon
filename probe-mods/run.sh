#!/usr/bin/env bash
set -euo pipefail
mode=$1
root=$PWD
cache="$HOME/.cache/horizon-native-stop"
base="$cache/base-$mode"; stage="$cache/stage-$mode"; target="$cache/target"
rm -rf "$base" "$stage"; mkdir -p "$base"
git archive HEAD | tar -x -C "$base"
if [ -L "$base/CLAUDE.md" ]; then rm "$base/CLAUDE.md"; cp "$base/AGENTS.md" "$base/CLAUDE.md"; fi
git rev-parse HEAD > "$base/.horizon-revision"
"$HOME/.cache/horizon-sample-mod/mixins/target/debug/horizon-mixins" apply --workspace "$base" --mods "$root/probe-mods" --out "$stage"
# Test-only workaround for upstream #3727: initialize Audio::share's new fields.
python3 - "$stage/crates/horizon-client-render/src/audio.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
start = text.index('    pub fn share(&self, owner: u8) -> Self {')
end = text.index('    /// Sync the volume options', start)
section = text[start:end]
old = '            preview: Default::default(),\n'
new = old + '''            #[cfg(feature = "audio")]
            default_seen: None,
            #[cfg(feature = "audio")]
            cards_seen: None,
            #[cfg(feature = "audio")]
            devices_seen: Vec::new(),
            #[cfg(feature = "audio")]
            device_ticks: 0,
'''
assert section.count(old) == 1
path.write_text(text[:start] + section.replace(old, new) + text[end:])
# Test-only workaround for the stale particle call on upstream main.
path = path.parents[2] / "horizon-client/src/game/creatures.rs"
text = path.read_text()
old = "self.lan_send_particles(&options, at, spread, 0.2, count);"
new = """self.lan_send_particles(super::lan_particles::QueuedParticle {
                dimension: self.dim.id(),
                options,
                state: None,
                at,
                spread,
                speed: 0.2,
                count: count as i32,
            });"""
assert text.count(old) == 1
path.write_text(text.replace(old, new))
PY
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="$target"
(cd "$stage" && cargo build --offline --profile fast -p horizon-client --bin horizon-client --bin horizon-ctl)
python3 probe-mods/test.py "$mode"
