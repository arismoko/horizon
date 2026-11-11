#!/usr/bin/env bash
# Run only after the preceding SDK build finishes: game builds stay sequential.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
export PATH="$ROOT/target/wholegame/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin:$PATH"
export RUSTUP_HOME="$ROOT/target/wholegame/rustup"
export RUSTUP_TOOLCHAIN=1.97.1-x86_64-unknown-linux-gnu
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8
for variant in Ma Mb Mc Mall; do
  stage="$ROOT/target/wholegame/kit-out/stage-$variant"
  python3 tools/function_site_bench/verify_stage.py "$stage" "$variant"
  export CARGO_TARGET_DIR="$ROOT/target/wholegame/kit-build-target-$variant"
  bash target/wholegame/kit/points-tick/build.sh "$stage" > "target/wholegame/kit-build-$variant.log" 2>&1
  destination="/box/shared/fnr-wholegame/bundle/builds/$variant"
  mkdir -p "$destination"
  cp -a "$stage/zc-game" "$destination/"
  cp "$stage/points.json" "$destination/"
  (cd "$destination" && find zc-game -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
  printf 'Built and bundled %s; functional smoke and census still required.\n' "$variant"
done
