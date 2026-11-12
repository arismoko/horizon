# Recipe filters

A content mod or a data pack can remove recipes, or swap one of their inputs or outputs, without knowing
their ids: a file `data/<ns>/recipe_filter/<name>.json` holds `{"filters": [...]}`. Each filter has match
keys, all of which must hold (a filter with none matches every recipe), and exactly one action:

| Key | Meaning |
|---|---|
| `id` | the recipe id; `*` matches any run of characters (`minecraft:*_planks`) |
| `type` | the recipe's `type`, with the same `*` (`mymod:*`) |
| `input` | an item id or `#tag` among the recipe's inputs |
| `output` | an item id among its outputs (anything under `result`, `results`, `output` or `outputs`) |
| `remove` | `true`: the recipe is not loaded |
| `replace_input` | `{"from": ..., "to": ...}`: every input that is `from` becomes `to` |
| `replace_output` | the same, for the outputs |

```json
{"filters": [
  {"id": "minecraft:*_planks", "remove": true},
  {"type": "mymod:*", "replace_input": {"from": "minecraft:iron_ingot", "to": "#c:ingots/iron"}}
]}
```

Filters run on each recipe after the packs are merged and its load conditions held, so they see the final
set and reach every recipe type, mods' own included: they work on the recipe's JSON, every string outside
`type`, `group`, `category` and the conditions (a `{"tag": ...}` object's tag reads as `#tag`). Every recipe
loader applies them: crafting, the stations, brewing, the recipe book and what players are sent. Files apply
in id order and each file's filters in order; a pack's file replaces a file of the same id, as any data file
does. A filter with an unknown key or not exactly one action is skipped with a warning, so a typo never
matches every recipe.
