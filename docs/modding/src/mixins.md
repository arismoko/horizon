# Mixins: `[[native.mixin]]`

A mixin changes one game function. You name the function and the place in it (its start, its return value,
one call inside it, or its whole body), and write your code there as a small Rust function, the **helper**.

Use a mixin when you need private fields, a change in the middle of a function, or a new body. To run code at
the start or end of a function with `mod_api` values, an [`[[inject]]` listener](injections.md) is enough.

## Example

This mixin runs at the start of `on_tick_rate_changed`, the server's `/tick rate` handler. It reads two private
fields of `Game` and can cancel the rest of the function. It is the check mod `fnr_alpha`, which CI builds.

`horizon-mod.toml`. The header is that of any native mod. `"HEAD"` and `"MIXINS"` are placeholders CI fills;
yours holds the 40-character commits `cargo horizon-mod new` wrote:

```toml
id = "fnr_alpha"
name = "Function replacement check: alpha"
version = "0.1.0"
description = "fn-replace: HEAD with cancel and a private field, a value chain, RETURN, OVERWRITE"
authors = []
api = "0.2"
side = "both"
capabilities = ["registries"]

[native]
game = "26.3"
horizon = "HEAD"
mixins = "MIXINS"

# HEAD, cancellable, reaching a private field (`ticks_until_autosave`) and the clock.
[[native.mixin]]
id = "head"
target = "horizon_client::game::autosave::Game<'_>::on_tick_rate_changed"
kind = "HEAD"
cancellable = true
helper = "src/mixins/head.rs"
bindings = [{ name = "game", ty = "&mut Game<'_>", value = "&mut *self" }]
```

`src/mixins/head.rs`, the helper:

```rust,ignore
fn head(game: &mut Game<'_>) -> ::core::ops::ControlFlow<()> {
    let rate = game.clock.rate();
    fnr_alpha::note(&format!(
        "HEAD on_tick_rate_changed: rate {rate}, ticks_until_autosave was {}",
        game.ticks_until_autosave
    ));
    if rate == 13.0 {
        fnr_alpha::note("HEAD cancels on_tick_rate_changed at rate 13");
        return ::core::ops::ControlFlow::Break(());
    }
    game.ticks_until_autosave = 4242;
    ::core::ops::ControlFlow::Continue(())
}
```

`src/lib.rs`, the mod's crate. Its package is `fnr-alpha`, so helpers call it as `fnr_alpha`:

```rust,ignore
//! The mod's own crate. Its `[[native.mixin]]` helpers (`src/mixins/`) are compiled into copies of the
//! game functions they patch, not into this crate.
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

/// Called by a helper: a helper may use its mod's crate.
pub fn note(what: &str) {
    eprintln!("fnr: alpha {what}");
}
```

`bindings` says what each helper parameter gets: here `game` gets `&mut *self`, the `Game` the function was
called on. With `cancellable = true` the helper returns `ControlFlow<()>`: `Break(())` makes the function
return at once, and `Continue(())` lets it run. For a function that returns `R`, the helper returns `ControlFlow<R>`, and `Break(value)` returns `value`.

When it builds, the build log shows `Function replacement: 1 functions patched by 1 mixins, without
rebuilding the game (…)`.

## Writing a helper

The helper file holds exactly one function: private, safe, with no attributes and no generic parameters
(except the `impl FnOnce` parameter of `WRAP_OPERATION`). The name is free, and the file has no `use` lines.

- It sees what the target's module sees: its items, imports and private fields (read the top of the target's
  file). That is why `Game` works in the helper and in `ty`. Write other names as full paths, such as
  `::core::ops::ControlFlow`. `crate::` is the game crate.
- A field or method it can't see needs a [bridge](bridges.md). A local variable needs a `$local:NAME:N` binding.
- It calls your mod's crate by its Rust name: the package name with `-` turned into `_`. Don't declare helper
  files as modules in `src/lib.rs`. Shared code that needs private game items goes in a [support module](support-modules.md).

## Kinds

| `kind` | Your helper runs | It returns |
| --- | --- | --- |
| `HEAD` | at the start of the function | `()`, or `ControlFlow<()>` with `cancellable = true` |
| `RETURN` | on the value the function returns (`$value`) | the value to return |
| `OVERWRITE` | instead of the whole body; one mod per function | the function's return type |
| `BEFORE_CALL`, `AFTER_CALL` | before or after one call in the body | `()` |
| `AFTER_ASSIGN` | after one statement that stores a call's result (`let x = f();`, `x = f();`) | `()` |
| `MODIFY_ARG`, `MODIFY_ARGS` | on one argument, or all arguments as a tuple, of one call | the new argument(s) |
| `MODIFY_CONSTANT` | on one literal in the body | the new value |
| `MODIFY_VARIABLE` | on one read or write of a local or parameter | the new value |
| `MODIFY_EXPRESSION_VALUE` | on the value of one call in the body (`$value`) | the new value |
| `WRAP_OPERATION` | around one call, which you get as `$operation` and may run or not | the call's type |

The other kinds act on one place in the body, which a `selector` names as source text. For a call, write the
callee without arguments: `autosave_interval` or `self.next_autosave_interval`. For
`MODIFY_EXPRESSION_VALUE`, write the whole expression: `autosave_interval(rate)`. `MODIFY_VARIABLE` uses
`variable` instead. `require` is how many matches the body has (inside `slice`, if set), and `ordinal` picks one, from 0.

## Fields

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | The row's name, unique in the mod. Errors name it as `mod:id`. |
| `target` | yes | The game function's path ([Naming the target](#naming-the-target)). |
| `kind` | yes | One of the kinds above. |
| `helper` | yes | The helper's file, a `.rs` path under `src/`. |
| `bindings` | when the helper has parameters | One `{ name, ty, value }` per helper parameter, in order. `name` and `ty` repeat the parameter. `value` is a target parameter or its borrow (`self`, `&*self`, `&mut *self`, `rate`), or a tool value (below). |
| `cancellable` | no | `HEAD`, `BEFORE_CALL`, `AFTER_CALL`, `AFTER_ASSIGN`: the helper returns `ControlFlow` and can end the function. |
| `selector`, `require`, `ordinal` | one-place kinds | Which call, how many matches, which match. |
| `variable` | `MODIFY_VARIABLE` | `{ name, ordinal, at = "LOAD" or "STORE", parameter = true for a parameter }`. It replaces `selector`; `require` and `ordinal` then count accesses. |
| `priority` | no | Orders rows of several mods at one place: **higher** runs first. The default is 100. (Loop slots run the other way.) |
| `result` | `MODIFY_ARGS`, and `AFTER_CALL` reading the result | The selected call's result type. Refused on other kinds. |
| `slice`, `argument` | no | `slice = { from = { selector, ordinal, require } }` narrows the search to a region; `argument` picks an argument. Both are in the tool's docs below. |

| Tool `value` | The helper gets |
| --- | --- |
| `$value` | The value being changed: the return value, the expression's value, or the argument, constant or variable. |
| `$arg0`, `$arg1`, … | An argument of the selected call (`WRAP_OPERATION`). |
| `$args` | All arguments of the selected call, as a tuple (`MODIFY_ARGS`). |
| `$receiver` | The receiver of a selected method call (`WRAP_OPERATION`); the first binding, and the operation's first parameter. |
| `$operation` | The selected call, as `impl FnOnce(args) -> result` (`WRAP_OPERATION`); the last binding. |
| `$local:NAME:N`, `$local_ref:NAME:N`, `$local_mut:NAME:N` | The `N`th local named `NAME` in the body, from 0: by value, `&` or `&mut`. |

horizon-mixins documents every kind and binding in full: [Mixin API](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/mixin-api.md)
and [advanced injections](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/advanced-injections.md).
These links are at the launcher's default `mixins` commit; read the same files at yours.

## Naming the target

A target is the function's module path, then its type and name:

- a method: `horizon_client::game::autosave::Game<'_>::on_tick_rate_changed`, where the module is the one
  holding the `impl` block and the type is written as there, lifetimes included;
- a free function: `horizon_client::game::commands::misc::millis`;
- a trait method: the module of the `impl Trait for Type` block, then `<Type as Trait>` as written, then the
  function: `horizon_client::game::level_access::<GameLevel<'_> as LevelAccess>::block_state`.

To get the module from the file, start from `crates/horizon-client/src/`, which is `horizon_client`. A folder
directly under `src/game/` that holds a `.game-area` file is an **area**, and an area is not a module.

So `src/game/world/autosave.rs` is `horizon_client::game::autosave` (`world/` is an area). But
`src/game/native_hooks/` has no `.game-area`, so `src/game/native_hooks/base.rs` is
`horizon_client::game::native_hooks::base`.

Any function of the game crate can take a mixin. A function in a crate below it, such as `horizon_world`, needs
the marker `#[horizon_mod_api_macros::replaceable]` in its source; ask for one in an issue on
[arismoko/horizon](https://github.com/arismoko/horizon/issues).

These functions are refused:

- generic over types or consts, or inside an `impl` block that is;
- taking `impl Trait`, returning `!`, or taking `self` by value;
- `const`, `async`, `unsafe` or `#[inline(always)]`;
- existing only under a `cfg`;
- a body that declares a `static`, thread-local, `macro_rules!` or `impl`;
- a body that uses a macro defined elsewhere in its crate.

The one generic function you can target is a [published loop's body](loops.md#mixins-on-the-loop).

## With other mods

Several mods' rows on one function all apply. An `OVERWRITE` must be the only mixin on its function,
from any mod including its own. An `[[inject]]` listener may share a function with mixins: head listeners run first, then the
mixed-in function, then return listeners.

## Errors

A row the build can't apply stops the build, naming the mod, the row and the reason. `<…>` is filled in.

| Code | Message | Fix |
| --- | --- | --- |
| `E_MIXIN_INVALID` | `<mod>:<row>: invalid target; name the game function's path` | Write the target. |
| | `… invalid helper; name a Rust file under src/ in this package` | Put the helper under `src/`, ending in `.rs`. |
| | `… invalid hook; a named hook is a mod_api::Hooks method; a mixin names its target` | Remove the row's `hook` (or `hook_version`) key and name a `target`, or implement the hook in `Hooks`. |
| | `… invalid target; a mixin targets a game function` | The target is in a mod you depend on. Call that mod's public functions instead. |
| `E_MIXIN_TARGET_MISSING` | ``<target>: no function `<name>` in <module> on <Type>``, or ``no crate `<crate>` in the game's source`` | Check the path against the source at your commit, module first. |
| `E_MIXIN_TARGET_AMBIGUOUS` | `<target>: several functions have this path (a cfg-gated pair, for one)` | Target another function. |
| `E_MIXIN_NOT_REPLACEABLE` | `<target>: a function below the game crate takes a replacement only when it carries #[horizon_mod_api_macros::replaceable]; …` | Target a game-crate function, or ask for the marker. |
| | `<target>: <reason>`, such as ``it is generic over types or takes `impl Trait` …`` | Target a caller or callee that can be copied. |
| `E_MIXIN_SOURCE` | `<target>: <why>` | The game's source could not be read: retry the build. |
| `E_OVERWRITE_CONFLICT` | from the tool: `OVERWRITE must be exclusive on <function>; competing injections: <mod>/<row>, …` | Keep only one of those mods installed. |
| `E_LOOP_ENTRY_REPLACED` | see [Published loops](loops.md#errors) | |
| `E_MIXINS_PATCH` | `tools/mixins/narrow-reference-rule.patch does not apply to horizon-mixins <commit>: <why>` | Not your mod: report it on the game. |

The tool's own codes, such as `E_MATCH_COUNT` and `E_UNSUPPORTED_SITE`, mean a selector, count or binding
does not fit the body; its docs list them. A helper with a wrong type is a compiler error under your mod's id.
