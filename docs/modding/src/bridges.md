# Bridges: `[[native.bridge]]`

A bridge gives your mixin helpers a getter, setter or caller for a field or method they can't see. The
launcher adds a method to the game's struct, and your helpers call it like any other method.

Use one when a [mixin helper](mixins.md#writing-a-helper) needs a field or method that Rust's privacy hides
from the target's module. For a value a hook or `[[inject]]` listener already gets, you need no bridge.

## Example

The check mod `fnr_support`, which CI builds, declares a mutable getter for `Game::ticks_until_autosave` and
uses it from a `HEAD` mixin. The manifest's header is as on [Mixins](mixins.md#example). The helper also
calls a [support module](support-modules.md) of the same mod, so the example keeps its row.

```toml
[[native.module]]
id = "runtime"
file = "crates/horizon-client/src/game/world/autosave.rs"
name = "fnr_runtime"
source = "src/runtime.rs"

[[native.bridge]]
id = "ticks"
kind = "GET_MUT"
target = "horizon_client::game::Game"
member = "ticks_until_autosave"

[[native.mixin]]
id = "head"
target = "horizon_client::game::autosave::Game<'_>::on_tick_rate_changed"
kind = "HEAD"
helper = "src/mixins/head.rs"
bindings = [{ name = "game", ty = "&mut Game<'_>", value = "&mut *self" }]
```

`src/mixins/head.rs`:

```rust,ignore
fn head(game: &mut Game<'_>) {
    let ticks = game.__horizon_mixins_fnr_support_ticks();
    *ticks = crate::game::autosave::fnr_runtime::bumped(*ticks);
    fnr_support::note(&format!("module and bridge: ticks_until_autosave {}", *ticks));
}
```

The bridge `ticks` of the mod `fnr_support` becomes the method `__horizon_mixins_fnr_support_ticks`. It returns
`&mut i64` because the field is an `i64`. (This field is visible from that module anyway; the check shows the shape.)

## The generated method

Its name is `__horizon_mixins_<mod>_<id>`: the mod's id and the row's id, lowercased, with `-` turned into `_`.
So use only letters, digits, `_` and `-` in a bridge id, and keep ids distinct after that change.

| `kind` | Method | `target` | `member` |
| --- | --- | --- | --- |
| `GET` | `fn(&self) -> &T` | The struct: `horizon_client::game::Game` | The field's name, or its index in a tuple struct as a string (`"0"`) |
| `GET_MUT` | `fn(&mut self) -> &mut T` | The struct | The field |
| `SET` | `fn(&mut self, value: T)` | The struct | The field |
| `INVOKE` | The method's own signature, renamed | The method, written like a [mixin target](mixins.md#naming-the-target) | leave it out |

`T` is the field's type: read it in the game's source, since a wrong guess is a compiler error. An `INVOKE`
bridge is called as `value.__horizon_mixins_<mod>_<id>(…)`, or `Type::__horizon_mixins_<mod>_<id>(…)` for a
function without `self`. No check mod covers `INVOKE` yet.

Name a field bridge's struct by the module that declares it, without lifetimes (`horizon_client::game::Game`).
Mixin helpers on game-crate functions can call the method. Your mod's own crate cannot, so
read the value in a helper and pass it to your crate.

## Fields

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | The row's name, unique in the mod. It is part of the method's name. |
| `kind` | yes | `GET`, `GET_MUT`, `SET` or `INVOKE`. |
| `target` | yes | The struct (field bridges) or the method (`INVOKE`). |
| `member` | field bridges | The field. |

Limits:

- The struct or method is in the game crate, `horizon_client`.
- A field bridge's struct may be generic over lifetimes, not types or consts.
- An `INVOKE` target's type and method are not generic.
- An `INVOKE` target's signature does not use an associated type through `Self`, such as `Self::Error`.

## What you will see

A bridge builds with the mixins, so the build log's `Function replacement:` line covers it.

## Errors

Every bridge error reads `E_MIXIN_BRIDGE_TARGET: <mod>:<row>: <why>`.

| Reason | Fix |
| --- | --- |
| `a field bridge's target is a struct path` | Write `module::Struct`. |
| `no module <module> in the game crate` | Check the module path at your commit ([Naming the target](mixins.md#naming-the-target)). |
| `no struct <Struct> in <module>` | Name the module the struct is declared in. |
| `<Struct> is generic over types` | Not possible on this route. |
| ``a field bridge names its `member` `` | Add `member`. |
| `<Struct> has no field <member>` | Check the field's name at your commit. |
| `unknown bridge kind <kind>` | Use `GET`, `GET_MUT`, `SET` or `INVOKE`. |
| `an invoker's target is a method or associated function of a type` | The target is a free function. Point `INVOKE` at a method. |
| `an invoker's type and method are not generic` | Not possible on this route. |

An `INVOKE` target that names no function gets `E_MIXIN_TARGET_MISSING` or `E_MIXIN_TARGET_AMBIGUOUS`, as on
[Mixins](mixins.md#errors).
