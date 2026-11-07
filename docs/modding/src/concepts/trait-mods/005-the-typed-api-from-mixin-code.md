<!-- chapter: The typed API from mixin code -->
<!-- separator: 1 -->

### The typed API from mixin code

A helper injection can use the typed API: bind the game or the `Host` and call attachments and handles. The
worked examples, and which shape to choose, are in [Injections](../../injections.md#make-your-injection-convertible).

A catalog hook's trait method, event and return make up its `hook_version` contract
([Named hook points](../../how-to/native-mods/011-named-hook-points.md#named-hook-points)). These per-hook contract versions are catalog metadata, not minimum game
versions. Catalog hooks, `Host` calls and raw `target` injections all build against the
[game's exact source revision](../../how-to/native-mods/005-native-mod-source-revisions.md#native-mod-source-revisions). Rebuild the mod when that contract changes.

Every `impl Hooks` must have `#[mod_api::horizon_mod]`. The attribute derives
subscriptions from the methods implemented in that block, so dispatch knows them at installation.
