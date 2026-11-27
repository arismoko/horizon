//! The `spawn_cluster_size` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `SpawnClusterSizeEvent`: how many mobs one natural spawn group may contain.
    /// Runs at the group-size comparison in `NaturalSpawner.spawnCategoryForPosition`
    /// (`EventHooks.getMaxSpawnClusterSize`), after the mob was finalized, added to the level
    /// with its passengers and counted by the spawn state, so `mob` is a resident mob: `host`
    /// reads it as the level holds it, and a mod that took it out in its join listener leaves a
    /// handle that no longer resolves.
    /// `vanilla` is the species' `Mob.getMaxSpawnClusterSize`; `size` starts there. Every part
    /// of the mob's type sees `size` as the parts before it left it, in dispatch order, and
    /// writes the one signed size the group is compared with: the group stops growing once
    /// its count reaches `size`, so a size of 0 or less ends it with the mob just spawned.
    /// The host is the read-only one: a listener runs on replayed ticks and must be a pure
    /// function of its inputs.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::spawn_cluster_size::ServerGame<'_>::native_spawn_cluster_size",
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        host = "read",
        contract = "listen",
        scope = "entity",
        key = "mob",
        neoforge = "SpawnClusterSizeEvent",
        version = 2
    )]
    fn spawn_cluster_size(mob: MobRef, vanilla: i32, size: &mut i32);
}
