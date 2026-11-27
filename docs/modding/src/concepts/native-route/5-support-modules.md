# Support modules: `[[native.module]]`

A support module puts shared Rust code in the function-replacement mirror of a game module. Its code
can use that game module's private items, and your mixin helpers can call it. The launcher compiles the
mirror against the prebuilt game; it does not edit or rebuild the original game source.

Use one for code that several helpers share and that needs the game's private items. Shared code that needs
only public items goes in your mod's own crate.

## Example

Declare `fnr_runtime` in the mirror of the autosave module. The full manifest and a helper that calls
it are on [Bridges](4-bridges.md#example).

```toml
[[native.module]]
id = "runtime"
file = "crates/sunrise-gameplay/src/game/world/autosave_server.rs"
name = "fnr_runtime"
source = "src/runtime.rs"
```

`src/runtime.rs` holds the module's items, with no `mod` wrapper:

```rust,ignore
pub fn bumped(ticks: i64) -> i64 {
    ticks + 1
}
```

A helper calls it by its path in the target's gameplay crate: `crate::game::server_side::autosave_server::fnr_runtime::bumped(*ticks)`. That
file is mounted as `game::server_side::autosave_server` ([Naming the target](2-mixins.md#naming-the-target)).

Inside the module, `super::` is the game module that received it, such as `super::ServerGame`. Make the items that
helpers call `pub`. The module may call your mod's crate; your mod's crate cannot call the module.

## Fields

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | The row's name, unique in the mod. |
| `file` | yes | The game source file whose module is mirrored, relative to the repository root: `crates/sunrise-gameplay-common/src/…`, `crates/sunrise-gameplay/src/…`, or `crates/sunrise-client-gameplay/src/…`. State files are `crates/sunrise-gameplay/src/game/server_side.rs` and `crates/sunrise-client-gameplay/src/game/client_side.rs` (see [Repinning native targets for gameplay crates](../../how-to/native-mods/16-repinning-side-module-targets.md) and [Moving a native mod to the gameplay crates](../../how-to/native-mods/17-moving-to-the-gameplay-crates.md)). |
| `name` | yes | The module's Rust name. Pick one the destination module does not use, with a prefix of your own, such as `fnr_`. |
| `source` | yes | Your file under `src/` with the module's items. |
| `visibility` | no | The module's visibility: `"pub(crate)"` (the default, enough for your helpers) or `"pub"`. |

## What you will see

A module builds with the mixins, so the build log's `Function replacement:` line covers it.

## Errors

The `sunrise-mod.toml:` errors come when the manifest is read; the coded ones come at build time.

| Message | Fix |
| --- | --- |
| ``E_MIXIN_MODULE_DESTINATION: <mod>:<row>: <file> is not a file of the game crate (`sunrise_client`); …`` | Name a file under `crates/sunrise-gameplay-common/src/`, `crates/sunrise-gameplay/src/`, or `crates/sunrise-client-gameplay/src/`. |
| `E_MIXIN_MODULE_SOURCE: <mod>:<row>: <parse error>` | Fix the Rust error shown in `source`. |
| `sunrise-mod.toml: module file must be a plain path` | Write `file` as a relative path without `..`. |
| `sunrise-mod.toml: module name must be an identifier` | Use a Rust identifier. |
| `sunrise-mod.toml: module source must be a file under src/` | Move the file under `src/`. |
| `sunrise-mod.toml: module visibility must be pub or pub(crate)` | Use one of the two, or leave it out. |
| `sunrise-mod.toml: empty or duplicate module id` | Give the row an id no other row uses. |
