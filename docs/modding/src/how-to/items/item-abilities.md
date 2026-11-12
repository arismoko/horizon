# Item abilities

An item's Part can change the answer to an ability question. The hook receives a read-only Host. Keep answers pure functions of the stack and ability so the client and server agree.

```rust,ignore
#[horizon_part]
impl ItemPart for CarvingTool {
    fn can_perform_action(&self, _: &dyn Host, event: &mut CanPerformActionEvent<'_>) {
        if event.ability == "minecraft:shears_carve" {
            event.can_perform = true;
        }
    }
}
```

Attach this Part to your item with the registrar's `item_part` method. This grants pumpkin carving only: `minecraft:shears_harvest` is a separate ability for mob and dispenser shearing. `minecraft:shield_block` permits blocking and uses the stack's blocking component, or the default component when absent. Leave unknown abilities unchanged.

The game asks these abilities at the following action sites:

| Ability | Consumers |
| --- | --- |
| `minecraft:shears_carve` | Pumpkin carving. |
| `minecraft:shears_harvest` | Shearing sheep, mooshrooms, snow golems and bogged; dispenser shearing; harvesting bee nest honeycombs. |
| `minecraft:shield_block` | Raising a blocking item and blocking attacks. |

`event.can_perform` starts with vanilla's answer. Set it to `true` to grant an ability or `false` to deny it. The answer controls permission, not the action's world effects or item wear.
