fn remove_touched_item(game: &mut Game<'_>, item: &str, canceled: &mut bool) {
    if item != "minecraft:diamond" {
        return;
    }
    let dimension = game.dim.id().to_owned();
    let entities = game.native_item_entities(&dimension, [-30000000.0, -2048.0, -30000000.0], [30000000.0, 2048.0, 30000000.0]);
    if let Some(mut entity) = entities.into_iter().find(|entity| entity.stack.item.as_ref() == item) {
        entity.stack.count = 0;
        let removed = game.native_set_item_entity(&dimension, entity.id, entity.stack);
        if let Ok(path) = std::env::var("PICKUP_PROOF") {
            let _ = std::fs::write(path, format!("removed={removed}"));
        }
        *canceled = std::env::var_os("PICKUP_CANCEL").is_some();
    }
}
