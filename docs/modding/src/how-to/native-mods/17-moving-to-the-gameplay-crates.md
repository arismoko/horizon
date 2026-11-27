# Moving a native mod to the gameplay crates

This guide walks through updating a native mod whose manifest or Rust source targets the old single-crate `sunrise_client::game` layout to the three gameplay crates. Every step gives exact before/after spellings, runnable commands, and a check to confirm the step is complete.

## Who needs this guide

Pure `mod_api` mods (mods that use only `mod_api::Hooks`, `Part`, `Host`, and data/content declarations) need no changes.

Run this check in your mod directory to see whether your mod is affected:

```sh
rg -n '^\[\[native\.(mixin|bridge|module|loop)\]\]|^\[\[internals\]\]|sunrise_client::(game|mod_content|vanillagen)' sunrise-mod.toml src/
```

If that command prints nothing, your mod is unaffected and you can stop here. If it prints any match, follow the steps below in order.

## What the game is now

- Gameplay code lives in three crates: `sunrise_gameplay_common` (shared gameplay), `sunrise_gameplay` (server gameplay), and `sunrise_client_gameplay` (client gameplay).
- The game state is three concrete types: `CommonGame<'g>` in `sunrise_gameplay_common::game`, `ServerGame<'g> { common, server }` in `sunrise_gameplay::game` (with `ServerState` in `sunrise_gameplay::game::server_side`), and `ClientGame<'g> { common, client }` in `sunrise_client_gameplay::game` (with `ClientState` in `sunrise_client_gameplay::game::client_side`).
- Code that runs on both sides lives in extension traits bounded by `SideGame<'g>` in `sunrise_gameplay_common`; `ServerGame<'g>` and `ClientGame<'g>` also `Deref` and `DerefMut` to `CommonGame<'g>`.
- Each gameplay crate builds its own function-replacement mirror (`sunrise_patch_gameplay_common`, `sunrise_patch_gameplay`, and `sunrise_patch_client_gameplay`) rather than one combined mirror.
- Dedicated server builds (`--no-default-features`) omit `sunrise-client-gameplay`, so targets in `sunrise_client_gameplay` exist only in desktop builds.

## Target map and SDK catalogs

Two references map old target strings to their new paths:

- [`docs/modding/migration/gameplay-crates-targets.tsv`](https://github.com/sunrise-org/sunrise/blob/main/docs/modding/migration/gameplay-crates-targets.tsv) (columns: `old_target`, `new_target`, `status` [`moved`, `renamed`, `unchanged`, `removed`, or `new`], `note`) and its companion notes [`docs/modding/migration/gameplay-crates-targets.md`](https://github.com/sunrise-org/sunrise/blob/main/docs/modding/migration/gameplay-crates-targets.md) in the Sunrise repository.
- The pinned build's catalogs on disk under `.sunrise/cache/<build>/build-data/functions.json` and `.sunrise/cache/<build>/build-data/internals.json`.

## Step-by-step migration

Work through these steps in order so the manifest and helpers stay aligned at each stage.

### 1. Pin the target build and list every affected declaration

Pin your mod to the target Sunrise build so `cargo sunrise-mod` fetches its `functions.json` and `internals.json` catalogs and reports every path that no longer resolves:

```sh
cargo sunrise-mod pin <build>
cargo sunrise-mod check
```

`pin` updates `[native] sunrise`, refreshes hashes for paths that still resolve, and prints every unresolvable path as `missing` without overwriting its old hash.

List all target and file rows in your manifest so you can track every rewrite:

```sh
rg -n '^(target|file) = ' sunrise-mod.toml
```

### 2. Retarget `[[native.mixin]]` targets

Do **not** blindly replace `sunrise_client::` with a single crate name:

- Each function now lives in `sunrise_gameplay_common`, `sunrise_gameplay`, or `sunrise_client_gameplay`.
- Files split by side gained `_common`, `_server`, or `_client` module stems, or moved under `server_side::`, `client_side::`, or `native_hooks::`.
- Inherent method targets use the exact `impl` receiver spelling in the owning file (`CommonGame<'_>`, `ServerGame<'_>`, `crate::game::ServerGame<'_>`, `ClientGame<'_>`, or `crate::game::ClientGame<'_>`).

Representative before/after rewrites:

| Before | After |
| --- | --- |
| `sunrise_client::game::server_side::autosave::Game<'_, crate::game::server_side::ServerState>::on_tick_rate_changed` | `sunrise_gameplay::game::server_side::autosave_server::crate::game::ServerGame<'_>::on_tick_rate_changed` |
| `sunrise_client::game::server_side::shield::Game<'_, crate::game::server_side::ServerState>::player_blocking` | `sunrise_gameplay::game::server_side::shield::ServerGame<'_>::player_blocking` |
| `sunrise_client::game::creatures::knockback::Game<'_>::knock_back_player` | `sunrise_gameplay::game::server_side::creatures_server::knockback::crate::game::ServerGame<'_>::knock_back_player` |
| `sunrise_client::game::server_side::native_hooks_server::block_entity_tick::Game<'_, crate::game::server_side::ServerState>::tick_mod_block_entity_loaded` | `sunrise_gameplay::game::native_hooks::block_entity_tick::crate::game::ServerGame<'_>::tick_mod_block_entity_loaded` |
| `sunrise_client::game::client_side::sign_edit::Game<'_, crate::game::client_side::ClientState>::draw_sign_editor` | `sunrise_client_gameplay::game::client_side::sign_edit::ClientGame<'_>::draw_sign_editor` |
| `sunrise_client::game::client_side::loading_screen::preflight_resources` | `sunrise_gameplay_common::game::preflight_resources` |
| `sunrise_client::game::level_access::<GameLevel<'_> as LevelAccess>::block_state` | `sunrise_gameplay_common::game::level_access::<GameLevel<'_> as LevelAccess>::block_state` |

Look up each old target in `docs/modding/migration/gameplay-crates-targets.tsv` (using your local Sunrise checkout path for `MAP`):

```sh
awk -F'\t' -v t="OLD_TARGET" '$1 == t { print $2 "\t" $3 "\t" $4 }' "$MAP"
```

If `status` is `removed` (empty `new_target`), read `note` for the refusal reason and alternative boundary, and see [What is gone and what to do instead](#what-is-gone-and-what-to-do-instead). To confirm an exact target string and `impl` receiver spelling directly against the pinned build, search `functions.json`:

```sh
rg -n '<function_name>' .sunrise/cache/<build>/build-data/functions.json
```

**Check:** confirm no `[[native.mixin]]` target still starts with `sunrise_client::game::`:

```sh
rg -n 'target\s*=\s*"sunrise_client::game::' sunrise-mod.toml
```

### 3. Update mixin helper parameter types and `bindings`

Update every `bindings` `ty` string in `sunrise-mod.toml` and the corresponding parameter type in each helper `.rs` file under `src/` to match the new target's receiver spelling.

| Before | After |
| --- | --- |
| `&mut Game<'_, crate::game::server_side::ServerState>` | `&mut crate::game::ServerGame<'_>` or `&mut ServerGame<'_>` |
| `&Game<'_, crate::game::server_side::ServerState>` | `&crate::game::ServerGame<'_>` or `&ServerGame<'_>` |
| `&mut Game<'_, crate::game::client_side::ClientState>` | `&mut crate::game::ClientGame<'_>` or `&mut ClientGame<'_>` |
| `&mut Game<'_>` (server target) | `&mut crate::game::ServerGame<'_>` or `&mut ServerGame<'_>` |
| `&mut Game<'_>` (common target) | `&mut crate::game::CommonGame<'_>` or `&mut CommonGame<'_>` |

Example helper before and after:

```rust,ignore
// Before
fn head(game: &mut Game<'_, crate::game::server_side::ServerState>) -> ::core::ops::ControlFlow<()> {
    if game.clock.rate() == 13.0 {
        return ::core::ops::ControlFlow::Break(());
    }
    ::core::ops::ControlFlow::Continue(())
}

// After
fn head(game: &mut crate::game::ServerGame<'_>) -> ::core::ops::ControlFlow<()> {
    if game.common.clock.rate() == 13.0 {
        return ::core::ops::ControlFlow::Break(());
    }
    ::core::ops::ControlFlow::Continue(())
}
```

When helpers explicitly spelled `ServerState` or `ClientState`, this mechanical rewrite is safe (review the diff afterwards, and adjust any file whose target module imports bare `ServerGame` or `ClientGame`):

```sh
sed -i "s/Game<'_, crate::game::server_side::ServerState>/crate::game::ServerGame<'_>/g" sunrise-mod.toml src/mixins/*.rs
sed -i "s/Game<'_, crate::game::client_side::ClientState>/crate::game::ClientGame<'_>/g" sunrise-mod.toml src/mixins/*.rs
```

Do **not** run a blanket `sed` on bare `Game<'_>`, because bare `Game<'_>` can correspond to `CommonGame<'_>`, `ServerGame<'_>`, or `ClientGame<'_>` depending on the target's owning crate.

**Check:** confirm no stale `Game<'` receiver remains:

```sh
rg -n "Game<'" sunrise-mod.toml src/mixins/
```

### 4. Rewrite `.side` field accesses and disjoint borrows of shared state

Side-specific state now lives on `.server` (for `ServerGame<'_>`) or `.client` (for `ClientGame<'_>`), not `.side`.

| Before | After |
| --- | --- |
| `game.side.<field>` in a server helper | `game.server.<field>` |
| `game.side.<field>` in a client helper | `game.client.<field>` |
| `game.<common_field>` borrowed alongside `game.side.<field>` | `game.common.<common_field>` alongside `game.server.<field>` (or `game.client.<field>`) |

Plain reads and writes of shared fields (such as `game.clock.rate()` or `game.world.game_time`) still work through `Deref`/`DerefMut` to `CommonGame<'_>`. Whenever a helper borrows a shared field at the same time as `game.server` or `game.client`, qualify the shared field explicitly with `game.common.` so Rust sees disjoint field borrows.

For helper files whose side is known, the `.side.` rewrite is mechanical:

```sh
# Run only on helpers whose receiver is ServerGame<'_>:
sed -i 's/\.side\./.server./g' src/mixins/<server_helper>.rs

# Run only on helpers whose receiver is ClientGame<'_>:
sed -i 's/\.side\./.client./g' src/mixins/<client_helper>.rs
```

**Check:** inspect any remaining `.side` reference in mixin helpers:

```sh
rg -n '\.side\b' src/mixins/
```

### 5. Retarget `[[native.bridge]]` targets and check mirror scope

Update every `[[native.bridge]]` `target` to the struct's or method's declaring gameplay crate and module:

| Before | After |
| --- | --- |
| `sunrise_client::game::Game` | `sunrise_gameplay_common::game::CommonGame`, `sunrise_gameplay::game::ServerGame`, or `sunrise_client_gameplay::game::ClientGame` (whichever owns `member`) |
| `sunrise_client::game::server_side::ServerState` | `sunrise_gameplay::game::server_side::ServerState` |
| `sunrise_client::game::client_side::ClientState` | `sunrise_client_gameplay::game::client_side::ClientState` |
| `INVOKE` target under `sunrise_client::game::…` | Same declaring path and receiver rule as Step 2 |

For the two side-state structs, a direct rewrite is safe:

```sh
sed -i 's|sunrise_client::game::server_side::ServerState|sunrise_gameplay::game::server_side::ServerState|g' sunrise-mod.toml
sed -i 's|sunrise_client::game::client_side::ClientState|sunrise_client_gameplay::game::client_side::ClientState|g' sunrise-mod.toml
```

Update every helper call site to match the new receiver field (for example, `game.side.__horizon_mixins_<mod>_<id>()` becomes `game.server.__horizon_mixins_<mod>_<id>()`).

Because each gameplay crate compiles its own mirror, a bridge declared on a struct in `sunrise_gameplay` is callable only from helpers targeting `sunrise_gameplay` (and likewise for `sunrise_gameplay_common` and `sunrise_client_gameplay`).

**Check:** confirm no `[[native.bridge]]` row still names `sunrise_client::game::`:

```sh
rg -n 'sunrise_client::game::' sunrise-mod.toml
```

### 6. Retarget `[[native.module]]` `file` paths and per-crate visibility

Update every `[[native.module]]` `file` from `crates/sunrise-client/src/game/…` to the file's owning gameplay crate and split filename (`_common.rs`, `_server.rs`, or `_client.rs` where split):

| Before | After |
| --- | --- |
| `crates/sunrise-client/src/game/world/autosave.rs` | `crates/sunrise-gameplay/src/game/world/autosave_server.rs` |
| `crates/sunrise-client/src/game/server_side.rs` | `crates/sunrise-gameplay/src/game/server_side.rs` |
| `crates/sunrise-client/src/game/client_side.rs` | `crates/sunrise-client-gameplay/src/game/client_side.rs` |

Also update:

- Helper call paths into the support module (for example, `crate::game::server_side::autosave::fnr_runtime::…` → `crate::game::server_side::autosave_server::fnr_runtime::…`).
- Type names inside the support module source (for example, `super::Game` → `super::ServerGame`, `super::ClientGame`, or `super::CommonGame`).

**Per-crate patch visibility:** each gameplay crate compiles its own patch crate (`sunrise_patch_gameplay_common`, `sunrise_patch_gameplay`, `sunrise_patch_client_gameplay`). A `pub(crate)` support module is visible only to helpers in that same gameplay crate. If helpers in two gameplay crates previously shared one support module:

- Move helpers that use only public types into your mod's own crate (`src/lib.rs`) and call them via `<your_mod_crate>::…`.
- Otherwise, declare a support module in each gameplay crate whose private items its helpers need, or expose shared items with `visibility = "pub"` when calling from a dependent crate.

**Check:** confirm no `[[native.module]]` `file` still points at `crates/sunrise-client/`:

```sh
rg -n 'file\s*=\s*"crates/sunrise-client/' sunrise-mod.toml
```

### 7. Regenerate `[[internals]]` (do not hand-edit hashes)

Internal item paths and hashes moved to the declaring gameplay crates and concrete game types (`CommonGame`, `ServerGame`, `ClientGame`). Do not edit `hash` strings by hand.

- If your mod uses compiler-discovered internals, regenerate the entire `[[internals]]` list:

  ```sh
  cargo sunrise-mod internals sync
  ```

- If your mod pins explicit `[[internals]]` entries manually, remove the old `sunrise_client::game::…` rows and add the canonical paths for the pinned build (pass `--kind <kind>` if a path is ambiguous):

  ```sh
  cargo sunrise-mod internals add '<new-canonical-path>'
  ```

  Confirm canonical keys in `.sunrise/cache/<build>/build-data/internals.json` when unsure.

**Check:** confirm `sunrise-mod.toml` has no remaining `sunrise_client::game::` entries:

```sh
rg -n 'sunrise_client::game::' sunrise-mod.toml
```

### 8. Update native Rust imports in `src/`

`sunrise_client::game` holds only the thin entry shell's layout, not gameplay modules or types.

- Replace `use sunrise_client::game::…;` imports in `src/` with the owning gameplay crate (`sunrise_gameplay_common::game::…`, `sunrise_gameplay::game::…`, or `sunrise_client_gameplay::game::…`).
- Replace `sunrise_client::mod_content` and `sunrise_client::vanillagen` with their defining crate `sunrise_sim` so headless and dedicated-server builds resolve them cleanly (`sunrise_client`'s re-exports from `sunrise_client_lib` are gated on `#[cfg(feature = "client")]`):

| Before | After |
| --- | --- |
| `use sunrise_client::game::…;` | `use sunrise_gameplay_common::game::…;`, `use sunrise_gameplay::game::…;`, or `use sunrise_client_gameplay::game::…;` |
| `sunrise_client::mod_content` | `sunrise_sim::mod_content` |
| `sunrise_client::vanillagen` | `sunrise_sim::vanillagen` |

Safe mechanical rewrite for `mod_content` and `vanillagen`:

```sh
find src -name '*.rs' -exec sed -i 's/sunrise_client::mod_content/sunrise_sim::mod_content/g; s/sunrise_client::vanillagen/sunrise_sim::vanillagen/g' {} +
```

**Check:** confirm no stale game, `mod_content`, or `vanillagen` import from `sunrise_client` remains:

```sh
rg -n 'sunrise_client::(game|mod_content|vanillagen)' src/
```

### 9. Audit client-only targets for dedicated server builds

Dedicated server builds (`--no-default-features`) omit `sunrise-client-gameplay` entirely. Any mixin, bridge, support module, or internal entry under `sunrise_client_gameplay` exists only on desktop builds.

Check whether your manifest references the client gameplay crate:

```sh
rg -n 'sunrise_client_gameplay|crates/sunrise-client-gameplay/' sunrise-mod.toml
```

If your mod runs on dedicated servers, keep server behavior in `sunrise_gameplay`, `sunrise_gameplay_common`, or `mod_api` hooks, and keep client-only patches separate from server-required code.

## What is gone and what to do instead

### Shared `SideGame<'g>` extension-trait defaults

Shared methods in `SideGame<'g>` extension-trait default bodies have no replacement point, and had none before either (when they lived in generic `impl<S: Side> Game<'_, S>` blocks): **0 replacement slots were lost here**. If you need to intercept behavior inside a shared extension-trait method, target a concrete caller or callee on `ServerGame<'_>`, `ClientGame<'_>`, or `CommonGame<'_>`, or use a typed `mod_api` hook.

### Removed and unselectable targets

118 physical replacement slots were removed (117 of which were selectable mixin targets; `run_with` was installer-only), and 10 surviving slots dropped out of the selectable function index, for **127 formerly selectable targets** that a mixin can no longer name directly:

| Category | Count | Examples | What to target instead |
| --- | ---: | --- | --- |
| Path-written trait-impl owners | 82 | `impl AllayLevel for crate::game::ServerGame<'_>` and other trait impls whose `Self` type is written with a module path | Target a concrete inherent caller or callee on `ServerGame<'_>`, `ClientGame<'_>`, or `CommonGame<'_>`, or use a typed `mod_api` hook |
| Imported / cross-crate trait-impl owners | 24 | `impl ServerEntity` for `Boat`, `Minecart`, `FallingBlock`, `PrimedTnt`, `Rocket`, `GeneralRow`; `impl RenderEntity` / `RenderEntityBounds` for `Boat`, `Minecart` | Target the server entity tick, save/load, portal, or render caller in `sunrise_gameplay` or `sunrise_client_gameplay`, or use an entity `mod_api` hook |
| Glob-imported trait-impl owners | 5 | `ChatTrustClientExt`, `CartLerpClientExt`, `PackStackServerExt` | Target the inherent caller in `remote_client` or `server_reload`, or a `mod_api` hook |
| Conditional trait declarations | 3 | `item_draw::<Host as SpecialHost>::{banner_texture, player_skin, shield_texture}` | Target the surrounding `item_draw` inherent function in `sunrise_client_gameplay` |
| Entry-shell startup and service functions | 4 | `dedicated_services`, `profile_resolver`, `run_with`, `session_verifier` (kept in `sunrise-client`, outside the gameplay crates) | Use `mod_api::Hooks` lifecycle callbacks (`register`, `setup`, `server_started`) instead of patching startup wiring |
| Surviving slots no longer selectable | 10 | 9 functions crossing a non-public import; 1 function body declaring a `static` | Target a selectable caller or callee in `functions.json`, or use a `mod_api` hook |

Look up any individual target in [`docs/modding/migration/gameplay-crates-targets.tsv`](https://github.com/sunrise-org/sunrise/blob/main/docs/modding/migration/gameplay-crates-targets.tsv) to see its `status` and `note`.

### New replacement points on `CommonGame`

The split also adds 424 new physical replacement slots (423 selectable, including 372 inherent methods on `CommonGame<'_>` and 9 forwarding or lowering adapters). Which of these newly exposed points remain replaceable is subject to change. Prefer a typed `mod_api` hook or a gameplay-side caller over targeting a newly exposed `CommonGame` helper or adapter.

## Closing checklist

Run these commands from your mod directory in order; every one must pass with no unresolved targets or stale hashes:

1. `cargo sunrise-mod pin <build>` — confirms `[native] sunrise` matches the target build and reports zero `missing` declarations.
2. `cargo sunrise-mod internals sync` — regenerates `[[internals]]` paths, kinds, and hashes for the pinned build (or run `cargo sunrise-mod internals add …` for explicit entries).
3. `cargo sunrise-mod check` — validates manifest rows, helper paths, and internal hashes against the pinned build.
4. `cargo sunrise-mod build` — compiles the mod package and verifies all mixins, bridges, and support modules against the target build (update any `fingerprint` reported with `E_FINGERPRINT` after verifying the target function body).

See also [Repinning native targets for gameplay crates](16-repinning-side-module-targets.md) and [Mixins: `[[native.mixin]]`](../../concepts/native-route/2-mixins.md).
