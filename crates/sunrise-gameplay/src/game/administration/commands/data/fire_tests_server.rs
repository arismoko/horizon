use super::Target;

pub fn target_exists(game: &crate::game::ServerGame<'_>, target: Target) -> bool {
    match target {
        Target::Other(id) => game.server.others.0.iter().any(|o| o.id == id),
        Target::Fangs(id) => game.common.fangs.list.iter().any(|f| f.id == id),
        Target::Spawner(id) => game.common.ominous_spawners.iter().any(|s| s.id == id),
        Target::Cloud(id) => game.common.breath_clouds.iter().any(|c| c.id == id),
        Target::Projectile(id) => game.common.creatures.projectiles.iter().any(|p| p.id == id),
        _ => false,
    }
}

pub fn tick_fire_entities(game: &mut crate::game::ServerGame<'_>) {
    game.server.ticked_entities_through = u32::MAX;
    game.tick_summoned_eyes();
    game.tick_fangs();
    game.tick_ominous_item_spawners();
    game.tick_potion_clouds();
    game.tick_projectiles_now().unwrap();
}
