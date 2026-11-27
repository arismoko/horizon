#!/usr/bin/env bash
# Check engine bindings separately from vocabulary generation; build only the source checker.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
# Game archives (game-build.yml), plus the SDK's Windows GNU target. Asking rustc
# for a target's cfgs needs neither its standard library nor a cross-linker.
configs=()
for target in x86_64-unknown-linux-gnu x86_64-pc-windows-msvc x86_64-pc-windows-gnu aarch64-apple-darwin; do
    # Start with the toolchain defaults; the archives also override CRT linkage.
    cfg=$("${RUSTC:-rustc}" --print cfg --target "$target")
    configs+=("$cfg")
    case "$target" in
        *-pc-windows-msvc) crt=+crt-static ;;
        *) continue ;;
    esac
    cfg=$("${RUSTC:-rustc}" --print cfg --target "$target" -C "target-feature=$crt")
    configs+=("$cfg")
done
if [[ -n ${SUNRISE_HOOK_WIRING_TOOL:-} ]]; then
    exec "$SUNRISE_HOOK_WIRING_TOOL" "$PWD" "${configs[@]}"
fi
exec cargo run --quiet --locked -p sunrise-mod-api-gen --bin sunrise-hook-wiring -- "$PWD" "${configs[@]}"
