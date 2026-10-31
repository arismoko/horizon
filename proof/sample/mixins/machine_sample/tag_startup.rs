fn sample_tag_startup(game: &Game<'_>) {
    let block = horizon_world::chunk::BlockState {
        name: "minecraft:oak_log".into(),
        properties: Default::default(),
    };
    let item = ItemStack::new("minecraft:oak_log", 1);
    println!(
        "TAG_STARTUP block={:?} item={:?}",
        game.native_server_block_in_tag(&block, "minecraft:logs"),
        game.native_server_item_in_tag(&item, "minecraft:logs"),
    );
}
