# How the native route works

This page is for the curious. You don't need it to write a mod on [the native route](native-route.md).

## Function replacement

The game is never rebuilt for a mod. When you build, the launcher downloads the prebuilt game for your
`horizon` commit and that commit's source. It copies each function your rows target out of the source. It
applies your mixins to the copies with horizon-mixins, and adds your loop slots, bridges and modules. Then it
compiles the copies against the prebuilt game, together with your mod's crate.

Each copy is installed through a **replacement cell**: a pointer the prebuilt game checks at the start of
every replaceable function. When the pointer is set, the function runs the installed copy instead. Every
function of the game crate gets a cell when the prebuilt game is built. A function in a lower crate gets one
only through `#[horizon_mod_api_macros::replaceable]`.

That is why some functions are refused: one copy must stand for the original, and a generic function would
need one copy per instance. Everything that can't be copied is refused by name before anything compiles.

Your helpers, slots and modules are compiled inside the copies, in the target's module. They are not part of
your mod's crate, which is compiled first. So they can call your crate, but your crate can't call them or a
bridge. A bridge's method is a trait implemented for the type, imported into the module of every copied
game-crate function.

Rows of several mods on one function go into one copy, ordered by the tool (by `priority` at one place). An `[[inject]]` listener on the
same function shares its replacement cell: head listeners run first, then the copy, then return listeners.

## horizon-mixins

[horizon-mixins](https://github.com/horizon-rust/horizon-mixins) is upstream's source-level mixin tool; the
kinds and bindings are its vocabulary. The launcher applies one small patch to it,
`tools/mixins/narrow-reference-rule.patch`, which accepts a call directly under `&` for the kinds that rewrite
only that call. `E_MIXINS_PATCH` means that patch no longer fits the tool's source.

Beside the docs linked from [Mixins](mixins.md#fields), the tool documents bridges in
[scopes and bridges](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/scopes-and-bridges.md).

## Published loops

The loop's entry, `tick_block_entities`, calls a generic body, `tick_block_entities_with`, with the slots.
With no mod, the body runs with an empty slot and costs nothing extra. With mods, the launcher compiles one
copy of the body for the installed slot types and installs it through the entry's cell. That is why
`OVERWRITE` on the entry is refused: it would bypass the loop.

A run of the loop started from inside a slot is the game's own loop, without slots. The enclosing pass's
working state is set aside and restored, so it carries on as before. The server logs once:
`Published loop block_entities.tick: it was entered again while it ran; it runs without its mods' slots`.

After a slot panics, that `Game` never runs the slots again. If something catches the panic and the `Game`
goes on, it logs once: `Published loop block_entities.tick: a mod's slot panicked; it runs without its mods'
slots`. The slots already made are dropped, each attributed to its own mod. If one mod's `Drop` panics, the
others are still dropped and the process does not abort.

## Why `[[native.state]]` is refused

A new field changes a type's size and the place of its other fields. Every function of the prebuilt game you
did not copy was compiled with the old layout and would keep reading the old places. Adding a field safely
would mean rebuilding the whole game for each set of mods, which this route exists to avoid.
