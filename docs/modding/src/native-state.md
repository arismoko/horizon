# Native state: `[[native.state]]` is refused

`[[native.state]]` would add a field to a game type. Horizon refuses it, because a new field would break the
prebuilt game, which is never rebuilt for a mod. A mod that declares it does not build:

```text
E_MIXIN_STATE_LAYOUT: <mod>:<row>: native state changes the prebuilt game's layout; use an attachment instead
```

Fix: remove the row and use an **attachment**. An attachment is a typed value the game keeps for you on an
entity, block entity, chunk, level or player, and saves with it when you give it a codec. It works the same
on native and wasm. See [server attachments](concepts/trait-mods.md#server-attachments) and
[querying attachment holders](how-to/attachments.md).

For data about the whole game, a [published loop](loops.md)'s slot value lives once per `Game`. A `static` in
your crate is shared by every `Game` in the process and outlives each world: key it by world and clear it yourself.
