fn sample_tag_proof(game: &Game<'_>, kind: &str) {
    if kind != "minecraft:item" {
        return;
    }
    let block = horizon_world::chunk::BlockState {
        name: "minecraft:oak_log".into(),
        properties: Default::default(),
    };
    let stone = horizon_world::chunk::BlockState {
        name: "minecraft:stone".into(),
        properties: Default::default(),
    };
    let item = ItemStack::new("minecraft:oak_log", 1);
    let stone_item = ItemStack::new("minecraft:stone", 1);
    println!(
        "TAG_PROOF block={:?} block_nonmember={:?} block_missing={:?} item={:?} item_nonmember={:?} item_missing={:?}",
        game.native_server_block_in_tag(&block, "minecraft:logs"),
        game.native_server_block_in_tag(&stone, "minecraft:logs"),
        game.native_server_block_in_tag(&block, "proof:missing"),
        game.native_server_item_in_tag(&item, "minecraft:logs"),
        game.native_server_item_in_tag(&stone_item, "minecraft:logs"),
        game.native_server_item_in_tag(&item, "proof:missing"),
    );
}
