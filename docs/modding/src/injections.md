# Injections

An `[[inject]]` attaches a typed listener to an injection point without compiling game code. Prefer a named hook or Part method when one exists. API points work on native and wasm, on engine functions.

To change a call or value inside a function, replace its body, or run code inside a hot loop, a native mod
takes [the native route](native-route.md): [`[[native.mixin]]`](mixins.md), [`[[native.loop]]`](loops.md),
[`[[native.bridge]]`](bridges.md) and [`[[native.module]]`](support-modules.md).

## Declaration

```toml
[[inject]]
id = "count"
at = "horizon_client::game::Game<'_>::tick"
kind = "head"
with = "count"
priority = 0
```

`id`, `at`, `kind` and `with` identify the declaration, function, point kind and listener. `priority` defaults to zero. An injection is always through the API; there is no `access` key. An optional signature `hash` pins the target.

| Kind | Contract |
| --- | --- |
| `head` | Before the body; mutable argument record and cancellation/result |
| `return` | After the body; mutable returned value |
| `replace` | Owns the function and returns its result without running the body |

Listeners receive API records. Mutable parameters are copied back. A `Game` method's API point also supplies the side's Host. A parameter without an API/Wire form refuses registration, naming its type, on both targets.

A staged SDK build supplies these sites. A plain, unstaged game has no point sites and
refuses API listeners at load by name. The launcher selects the SDK build when any
enabled mod, native or wasm, declares an `[[inject]]` row. Each point is a replaceable
engine function with an API record: its head loads one replacement cell, and the
dispatcher for a listened point is installed through that cell.

## Ordering

Head cancellation skips the body and return point, but every head listener still runs and may clear cancellation. Return listeners run only after the body. Replacement listeners run after other API listeners; otherwise priority descends, then dependency registration order, with inject ID breaking ties within a mod. Only one replacement may own a point; disabling it restores the original body. Calling the replaced function from its replacement re-enters the point, not the original body.

Native code that needs game internals uses [`[[native.mixin]]`](mixins.md). The removed `access` key is refused by name: `access = "internals"` points to `[[native.mixin]]`, and `access = "api"` says to remove the line.

A library mod publishes its own hooks and functions for other mods to use.

## Refusals

| Code or message | When |
| --- | --- |
| `E_INJECT_INVALID` | A field is malformed or was removed; the field is named. `access = "internals"` points to `[[native.mixin]]`; `access = "api"` says to remove the line. |
| `E_INJECT_NO_SLOT` | A removed kind (`before_call`, `after_call`, `modify_value`, `wrap_call`, `wrap_if` and the other source-level kinds): the message names the `[[native.mixin]]` form to use. Also a function whose record the API cannot carry. |
| `E_INJECT_MISSING` | `at` names no function the pinned game lists. |
| `E_INJECT_CHANGED` | The pinned `hash` no longer matches the function's signature; run `cargo horizon-mod inject update`. |
| "no API listener at …" | The function is listed but takes no API listener. The reason follows: a parameter with no API type, named; or "the function has no point site: it takes no replacement (…)" for a generic function, a function in a generic impl, or one taking `impl Trait`. |
| "no replace listener at …" | A `replace` at a function whose return value has no API form. |
| "this build has no point sites; start the game through the launcher's mod build" | The game was not built as the SDK. |

## Ask for the hook

A gap is a hook request. File one as an issue on
[arismoko/horizon](https://github.com/arismoko/horizon/issues) with the contract below.
Copy this, and fill every field:

```markdown
**Hook request: <short name>**

- **What the mod needs:** <one sentence: the moment, and what the mod does there>
- **NeoForge 26.3 counterpart:** <event or method name, or "none">
- **Vanilla class and method:** <`net.minecraft…Class.method`, the call the hook wraps>
- **The injection it replaces:** <the `[[inject]]` table, `at`, `kind` and callback signature>
- **Mod:** <name, repository or `.hmod`>
- **Side:** <server, client or both>
- **Contract wanted:** <listen, cancel, replace a value, collect; what a listener may change>
```

A hook with a NeoForge counterpart takes its name, side and contract. One without is welcome and names the
mod mixin or use it replaces. State the vanilla method: the hook fires where vanilla calls it.

API points need the patchable (staged) build used by the launcher’s mod build; the plain game refuses them by mod and target name.
