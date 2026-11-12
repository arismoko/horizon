# Native mixins

Hooks and Parts are the shared API for native and wasm mods. Implement `mod_api::Hooks` or the appropriate Part method when it covers the behaviour you need.

A native mod can change game code with `[[native.mixin]]` rows. The launcher compiles a patched copy of each targeted function against the pinned prebuilt game and installs that copy. It does not rebuild the game. Wasm mods cannot declare native code transformations.

Each row has `id`, `target`, `kind` and a `helper` Rust file under `src/`. Use the pinned horizon-mixins vocabulary: HEAD, RETURN, BEFORE_CALL, AFTER_CALL, AFTER_ASSIGN, MODIFY_ARG, MODIFY_ARGS, MODIFY_CONSTANT, MODIFY_VARIABLE, MODIFY_EXPRESSION_VALUE, WRAP_OPERATION or OVERWRITE. Bindings and selectors describe the values the helper needs. Unsupported selectors or control flow refuse the build; do not silently choose another call.

```toml
[[native.mixin]]
id = "longer_autosave"
target = "horizon_client::game::Game::next_autosave"
kind = "MODIFY_EXPRESSION_VALUE"
helper = "src/longer.rs"
```

Native draw functions such as `horizon_mod_render::render::world_drawn`, `hand_drawn` and `world_views` are reached with a HEAD mixin. Engine hook functions named `native_*` are replaceable too. Mixins operate on native Rust values, including live mutable references, not API copies.

The patch-library support tables are `[[native.bridge]]` and `[[native.module]]`. They do not
compile game crates. Unsupported bridge or module declarations are refused by name.
`[[native.state]]` is refused with `E_MIXIN_STATE_LAYOUT`: the prebuilt game's layout is fixed;
use attachments for per-object mod data. Declare private engine items through the fingerprinted
`internals` list. Pin the game and tool revisions and rebuild the mod when those pins change.

The pinned commit needs a published SDK (`E_NATIVE_NO_RELEASE`) for a supported host
(`E_NATIVE_HOST`). A mod's own build script is `E_NATIVE_BUILD_SCRIPT`; ship generated source
instead. Custom mod or SDK feature selections are `E_NATIVE_FEATURES`; use the SDK's feature set.

## Refusals

A build that cannot apply a row stops and names the mod, the row and the reason. Nothing falls back to another route.

| Code | When |
| --- | --- |
| `E_MIXIN_TARGET_MISSING` | `target` names no function of the pinned game. |
| `E_MIXIN_TARGET_AMBIGUOUS` | `target` matches more than one function. |
| `E_MIXIN_NOT_REPLACEABLE` | The function takes no replacement: it is generic over types or consts, sits in a generic impl, takes `impl Trait`, is `const`, `async` or `unsafe`, takes `self` by value, is `#[inline(always)]`, or is a function below the game crate without `#[horizon_mod_api_macros::replaceable]`. Ask for the marker in an issue or pull request on the game. The reason is named. |
| `E_MIXIN_SOURCE` | The pinned source of the target cannot be read or copied. |
| `E_MIXIN_INVALID` | A row's field is malformed; the field is named. |
| `E_OVERWRITE_CONFLICT` | Two mods `OVERWRITE` one function (from the mixin tool). |
| `E_MIXIN_STATE_LAYOUT` | `[[native.state]]`: use attachments. |
| `E_MIXIN_BRIDGE_TARGET`, `E_MIXIN_MODULE_DESTINATION`, `E_MIXIN_MODULE_SOURCE` | A `[[native.bridge]]` or `[[native.module]]` row names something the patch library cannot build. |
| `E_MIXINS_PATCH` | The carried patch does not apply to the pinned mixin tool. |

An `[[inject]]` listener and a mixin may share a function: the API head listeners run first, then the patched copy, then the API return listeners. A [published loop](loops.md) is filled with `[[native.loop]]`.

Prefer a hook request when the behaviour should also work on wasm. A native mixin is trusted code, not a sandbox.

## Pinned tool documentation

The vocabulary is horizon-mixins' pinned vocabulary. Read its [Mixin API](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/mixin-api.md), [advanced injections](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/advanced-injections.md), and [scopes and bridges](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/scopes-and-bridges.md) for selector and binding contracts.

The launcher applies the carried `tools/mixins/narrow-reference-rule.patch` to the pinned tool, keyed by its hash and checked against the exact source text. If it cannot apply, the build refuses by name. It narrows reference handling for supported call expressions; unsupported control flow still refuses rather than choosing another call.
