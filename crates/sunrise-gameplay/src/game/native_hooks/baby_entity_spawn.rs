/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// Two animals bred and the baby they made is about to join the world
    /// (NeoForge's `BabyEntitySpawnEvent`, vanilla's
    /// `Animal.spawnChildFromBreeding`): `parent` and `partner` are entity
    /// ids, `player` and `player_entity_id` the UUID and entity id of the
    /// player who fed them into love (`getCausedByPlayer`, None when none).
    /// `child` (`BabyEntitySpawnChild<'_>`) lends the single optional breeding
    /// slot holding the constructed baby (inspect, edit in place, replace or
    /// take/clear). Set `cancel` for no baby: matching NeoForge's
    /// `Animal.spawnChildFromBreeding`, both parents are reset to age 6000 and
    /// love 0, and there is no baby, no experience, no `animals_bred`
    /// statistic, no advancement and no heart particles. Not fired for an egg
    /// (turtle, frog, sniffer). Villagers do not post this animal-only event,
    /// matching NeoForge's documented `Animal.spawnChildFromBreeding` posting
    /// site; a rejected villager join instead leaves the child's bed free. On
    /// the server. API 395.
    pub fn native_baby_entity_spawn(&mut self, call: mod_api::__private::BabyEntitySpawnCall<'_>) {
        self.fire_baby_entity_spawn(call);
    }
}
