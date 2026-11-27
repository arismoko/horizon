/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `MobSpawnEvent.SpawnPlacementCheck`: after `SpawnPlacements.checkSpawnRules`,
    /// `default_result` is the effective predicate result, including native registrations
    /// even for vanilla types. `spawner` is the block that asked, if any. `random` is the
    /// same live stream the predicate used, lent only for the call. `Default` keeps the
    /// predicate result; `Succeed` and `Fail` override it without undoing draws. Server-side.
    pub(in crate::game) fn native_mob_spawn_placement_check(
        &self,
        call: mod_api::__private::MobSpawnPlacementCheckCall<'_>,
    ) {
        self.fire_mob_spawn_placement_check(call);
    }
    /// `MobSpawnEvent.PositionCheck`: the freshly created mob of type `kind`
    /// may stand at `pos` in `level` for `reason` (`Mob.checkSpawnRules` and
    /// `checkSpawnObstruction`). `spawner` is the block that asked, if one
    /// did; `mob` is a scoped read-only [`mod_api::UnspawnedMobView`] of the
    /// mob itself. `result` starts as `Default`: vanilla's two checks decide;
    /// `Succeed` or `Fail` skips them. Server-side.
    pub(in crate::game) fn native_mob_spawn_position_check(
        &self,
        call: mod_api::__private::MobSpawnPositionCheckCall<'_>,
    ) {
        self.fire_mob_spawn_position_check(call);
    }
}
