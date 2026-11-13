# Native mods: building and shipping

A native mod is Rust source in a `.hmod`. The launcher compiles it on the player's machine, with its own pinned Rust
toolchain, against the prebuilt game of the exact Horizon build the mod pins in `[native]`. The game itself is never
rebuilt for a mod: the mods are compiled and linked against that prebuilt game, and the instance then plays the
result.

Native code runs with the player's rights, outside any sandbox. The first install shows a trust notice, and the
launcher builds native mods only on an install that allows native code (`--trust-native-code`, revocable in
**Settings > Storage**). A mod that must run in a sandbox is a [WebAssembly mod](wasm-mods.md).

The pages in this section follow a mod from its source to a player's game and a server:

<!-- section-pages -->
