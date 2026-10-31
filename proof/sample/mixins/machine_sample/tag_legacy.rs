fn sample_tag_legacy(game: &Game<'_>, context: &NativeLifecycleContext) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ONCE: AtomicBool = AtomicBool::new(false);
    if ONCE.swap(true, Ordering::Relaxed) {
        return;
    }
    let block = horizon_world::chunk::BlockState {
        name: "minecraft:oak_log".into(),
        properties: Default::default(),
    };
    let item = ItemStack::new("minecraft:oak_log", 1);
    println!(
        "TAG_LEGACY block={:?} item={:?}",
        game.native_block_in_tag(context, &block, "minecraft:logs"),
        game.native_item_in_tag(context, &item, "minecraft:logs"),
    );
}
