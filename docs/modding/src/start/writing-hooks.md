# Writing a trait mod

`cargo horizon-mod new <path> --template native` pins an exact Horizon commit (the newest prebuilt game's,
or `--horizon <commit>`'s; `cargo horizon-mod update` repins) and writes a flat project: `horizon-mod.toml` with `[native]`, `Cargo.toml`, `src/lib.rs`, and empty `assets/`
and `data/` directories. The template greets each player who joins. Build it with
`horizon-launcher native-build <path> --trust-native-code`; `cargo horizon-mod build` packages the same
project into a `.hmod` without compiling it ([the manifest](../reference/manifest.md)).

The root Rust package depends on `horizon-mod-api` (lib `mod_api`), never on the game. Its root `src/lib.rs` marks one
`impl Hooks` with the attribute:

```rust,ignore
use mod_api::{self, Hooks};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    // Implement the hooks this mod needs.
}
```

The attribute generates the `hooks()` factory and derives optional hook subscriptions from this impl.
The mod type implements `Default`; do not write a second factory. Keep the marked impl at the crate root.
Rustc's "cannot find function `hooks`" at the launcher's generated call means the marked implementation
is not at the crate root. The launcher links this package into a generated game executable and calls
the generated factory in dependency order (a mod after the mods it depends on, else in bundle order).
A mod with no raw injections skips horizon-mixins; typed hooks need no injection declarations.
Dependencies follow the pinned Horizon `Cargo.lock` rules in [The mod's crate](../how-to/native-mods/01-the-mods-crate.md).
