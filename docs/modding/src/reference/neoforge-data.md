# NeoForge data files

This is a scoped audit of server data, not a claim that carrying a file into a
package executes it. The paths and examples below identify the readers and
fixtures checked; the modifier gaps are tracked separately.

| Data | Result and reader | Example or evidence |
| --- | --- | --- |
| Convention item tags | Loaded for content roots: `mod_content::build` in `crates/horizon-client/src/mod_content.rs` merges tag files; `content_namespaces::tag_namespaces` in `crates/horizon-items/src/content_namespaces.rs` includes `c` for content roots, and `items::load_tag_lists` in `crates/horizon-items/src/items.rs` reads and resolves them. Unknown item entries are filtered by `Known::keeps` in `mod_content.rs`. | `data/c/tags/item/gems/glint.json`: `{"values":["minecraft:diamond"]}`. The `glint_mod` fixture and `a_data_mod_loads_unloads_and_loads_again` test in `crates/horizon-client/src/mod_content_tests.rs` check the resulting `c:gems/glint` membership. |
| Structure modifiers | Not loaded by any reader found today. `mod_content::build` copies ordinary files to its overlay; `worldgen_entry` in that file recognizes worldgen bundles, not structure modifiers. A workspace search for `structure_modifier` found no reader. Copying is not applying. | No verified in-tree example. [#1269](https://github.com/arismoko/horizon/issues/1269) tracks loading and applying them. |
| Global loot modifiers | Not loaded by any reader found today. `mod_content::build` copies ordinary files to its overlay, but a workspace search for `loot_modifiers` found no loader. The `native_loot_drops` code hook in `crates/horizon-mod-vocab/src/hooks/base.rs` is not a data-file reader. | No verified current-format in-tree example. [#1270](https://github.com/arismoko/horizon/issues/1270) tracks loading and applying them. The old `global.json` in `carry::tests::claims_the_namespaces_its_files_name` is a carry classification fixture, not proof of current-format loading. |
| Title screen splashes | Loaded from the resource stack: `splashes::pool` in `crates/horizon-client/src/splashes.rs` reads every namespace's top `texts/splashes.txt` (vanilla, content mods and enabled resource packs), joined in namespace order with duplicates kept, as NeoForge's `NeoForgeSplashHooks.loadSplashes`, on each resource reload. Lines are trimmed plain text, empty lines are kept and the hidden line is dropped. When a resource pack or a mod provides `minecraft:texts/splashes.txt`, only that file is read while `skipModdedSplashesIfVanillaModified` (in `horizon-extras.txt`, on by default) is on. | `assets/<modid>/texts/splashes.txt`, one splash per line. `a_content_mods_lines_follow_vanillas` and `a_packs_own_minecraft_splashes_replace_the_pool` in `splashes.rs`. |

The shared load-condition evaluator in `crates/horizon-world/src/conditions.rs`
applies to recipes, loot tables, item modifiers, predicates, enchantments,
jukebox songs, instruments, damage types, advancements, synchronized registry
JSON, world-table tags (including the direct loot and damage readers,
synchronized registry tags and function tags), and native item/block data maps.
The [conditional-data contract](../how-to/native-mods.md#conditional-data)
describes these file types and their reload lifetimes.

Ordinary JSON resources select the winning pack file before evaluating its
conditions. A false condition omits that winner without exposing a lower pack's
file. [Registered conditions](../concepts/load-conditions.md) can also refuse
reload on bad arguments or unknown names in an owned namespace. Tag resources and data maps evaluate each layer before
merge, `replace`, or `remove`; an omitted layer cannot clear lower entries.
Content-root composition follows the same tag rule, including block/biome tag
bundles. The damage registry remains frozen after world open; reloads reevaluate
its tags. Generated item default components retain their prototype lifetime.

Supported forms are `neoforge:mod_loaded`, `neoforge:true`, `neoforge:not`
(`value`), and `neoforge:and` / `neoforge:or` (`values`), inside a
`neoforge:conditions` array. Every condition must hold. An unknown kind in a
namespace where a loaded mod registered conditions refuses reload, naming the
file, condition and owner. Other unknown kinds omit the file with one named
error-level log entry, including beneath `not` or `or`. Files without the array load normally.

`mod_loaded` reads an immutable snapshot of the owning generation's server-enabled
mod IDs, including code-only native and WebAssembly mods, plus `minecraft`.
Client-only, disabled and refused packages are absent. Namespaces do not imply a
loaded mod. Changed enabled IDs participate in cache identity and are published
with successful reloads; failed preparation preserves the old generation, and
retained readers keep their old snapshot. Recipe condition behavior is unchanged.
This shared contract does not add readers for the two modifier kinds above.
