<!-- chapter: Changing the game's own code -->
<!-- separator: 1 -->

## Changing the game's own code

Hooks, Parts and `[[inject]]` listeners cover most mods, and they work the same on native and wasm. When no hook
fits and you need to change what a game function itself does, a native mod can take [the native route](../../native-route.md):
tables under `[native]` in `horizon-mod.toml`, applied to the prebuilt game without rebuilding it. The route is
unstable on purpose: it sees the game's real types and is pinned to the exact game build.

| Table | Use it to | Page |
| --- | --- | --- |
| `[[native.mixin]]` | run code at the start or end of a game function, change a value or a call inside it, or replace its body | [Mixins](../../mixins.md) |
| `[[native.loop]]` | run your code before and after each entry of a hot loop the game publishes, such as the block entity tick | [Published loops](../../loops.md) |
| `[[native.bridge]]` | read, write or call a private field or method of a game type from your mixin code | [Bridges](../../bridges.md) |
| `[[native.module]]` | add a module of your own code inside a game source file, for your mixin code to share | [Support modules](../../support-modules.md) |
| `[[native.state]]` | add a field to a game type: refused, use an attachment | [Native state](../../native-state.md) |
