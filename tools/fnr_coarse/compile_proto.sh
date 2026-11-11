#!/usr/bin/env bash
# compile_proto.sh SDK LIB.rs OUT.rlib: the launcher's patch-crate rustc invocation
# (launcher/src/native_mods/{patch,prebuilt}.rs at no-points-api-points 6c4d6a7e25), timed.
set -euo pipefail
sdk=$(realpath "$1"); lib=$2; out=$3
args=(--edition 2024 -C prefer-dynamic -C opt-level=3
  -L "dependency=$sdk/deps" -L "dependency=$sdk/host" -L "native=$sdk/native" -L "dependency=$(dirname "$out")"
  --crate-type rlib --crate-name horizon_patch --cap-lints allow -o "$out" "$lib")
while read -r f; do args+=(--cfg "feature=\"$f\""); done < <(python3 -c "import json,sys; print('\n'.join(json.load(open(sys.argv[1]))['game_features']))" "$sdk/horizon-prebuilt.json")
lib_of() { ls "$sdk"/deps/lib$1.so 2>/dev/null || ls "$sdk"/deps/lib$1-*.rmeta "$sdk"/deps/lib$1-*.rlib 2>/dev/null | head -1; }
for c in horizon_client $(python3 -c "import json,sys; print(' '.join(c for c in json.load(open(sys.argv[1]))['facade_crates'] if c!='horizon_client'))" "$sdk/horizon-prebuilt.json"); do
  f=$(lib_of "$c" || true); [ -z "$f" ] || args+=(--extern "$c=$f")
done
start=$(date +%s.%N)
rustc "${args[@]}"
echo "rustc $(python3 -c "print(round($(date +%s.%N)-$start,2))") s"
