# An entity's items

`entity.capability(host, &ENTITY_ITEMS, ())` (NeoForge 26.3's `Capabilities.Item.ENTITY`) is an entity's items, None for a
stale handle and an entity outside the level in place. The game answers it for a player with their inventory
(`PlayerInventoryWrapper`), indices as vanilla's `Inventory` numbers them: 0..36 the main inventory (0..9 the hotbar),
36..40 the armour from feet to head, 40 the off hand. Each index is the player's `ItemPlace::PlayerSlot` and takes only
what fits there: an armour slot one of what its `equippable` names it for (`ArmorSlot`: its allowed entities hold the
player by id or by tag), and bound armour comes off only in creative. `insert_any`, without an index, goes where
`Inventory.add` puts a stack: onto the same item in the selected slot, the off hand and the main inventory in order,
then into its empty slots, never the armour. For a chest or hopper minecart, a chest boat and a chested donkey, mule or
llama the game answers with their container's slots (`ItemPlace::EntitySlot`). A handler kept past its entity's removal
or its world's close reaches nothing. `r.entity_capability(&ENTITY_ITEMS,
"mymod:robot", |host, robot, ()| …)` answers it for a mod's entity type, and `r.entity_capability_before_game` ahead of the
game's own, asked as block entities' providers are. Giving, taking and counting a player's items go through it:

```rust
// A trade: each offer is (index, the exact stack it must still hold). A stale offer or no
// room for one aborts the transaction, and every move in it is put back.
let (a, b) = (seller.capability(host, &ENTITY_ITEMS, ())?, buyer.capability(host, &ENTITY_ITEMS, ())?);
host.transaction(|host, tx| {
    for (from, to, offer) in [(&a, &b, &seller_offer), (&b, &a, &buyer_offer)] {
        for (index, stack) in offer {
            let offered = ItemResource::of(stack);
            let still = from.resource(host, *index).as_ref() == Some(&offered)
                && from.amount(host, *index) == stack.count();
            if !still
                || from.extract(host, tx, *index, &offered, stack.count()) != stack.count()
                || to.insert_any(host, tx, &offered, stack.count()) != stack.count()
            {
                return Err(());
            }
        }
    }
    Ok(())
})
```

Cauldron interactions outside a transaction keep vanilla's effect order. Inside an open
transaction, their statistic award (including `player_stat_award`), sound and overflow
item toss/spawn wait until the outermost successful commit; an abort emits none of them.
