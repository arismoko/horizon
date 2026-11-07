<!-- chapter: Item model values -->
<!-- separator: 1 -->

Conditional, numeric and categorical item model properties and item tint sources
receive an API-owned `ItemStack` snapshot. The engine's resolver keeps its own
`StackView`; that engine type is not part of `mod_api`. This applies to nested
bundle contents as well as the outer item being drawn.

| Hook | Snapshot | Runtime selection | Answer |
| --- | --- | --- | --- |
| `item_model_property` | `event.stack: &ItemStack` | `event.bundle_selected: i32` | `event.value: bool` |
| `item_model_range_property` | `event.stack: &ItemStack` | `event.bundle_selected: i32` | `event.value: f32` |
| `item_model_select_property` | `event.stack: &ItemStack` | `event.bundle_selected: i32` | `event.value: &mut Option<String>` |
| `item_tint_source` | `event.stack: &ItemStack` | `event.bundle_selected: i32` | `event.color: u32` |

`bundle_selected` is the runtime bundle selection index: **-1 means none**. It is
not a saved component and is deliberately separate from the stack's component
patch. The patch contains explicit overrides and removals, not merged defaults.
The host is `&dyn ClientHost` for each callback. Only answer property names or
sources the mod owns; unanswered callbacks retain their existing defaults.

| Former resolver-view access | Owned API access |
| --- | --- |
| `event.stack.item` | `event.stack.item()` returns `ItemKey`; retain keys for comparisons |
| `event.stack.count` | `event.stack.count()` |
| `event.stack.patch` | `event.stack.components()` returns an optional owned NBT patch; use `event.stack.get(&descriptor)` for a typed component |
| `event.stack.bundle_selected` | `event.bundle_selected` |

A native or wasm listener sees the same owned values. Snapshot construction occurs
only after the subscription check, so a model resolution with no listeners does
not allocate or encode a snapshot. An empty or unresolved resolver view does not
dispatch an invalid stack; its property or tint retains the unanswered default.
