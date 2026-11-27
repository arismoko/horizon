use super::*;
use mod_api::DragonRespawnStage;

impl crate::game::ServerGame<'_> {
    /// The End's dragon fight is about to tick. Set `canceled` and it does
    /// nothing this tick. Server-side.
    pub(in crate::game) fn native_dragon_fight_tick(
        &mut self,
        level: LevelRef,
        canceled: &mut bool,
    ) {
        self.fire_dragon_fight_tick(level, canceled);
    }
    /// The End's dragon respawn reached `stage`. Server-side.
    pub(in crate::game) fn native_dragon_respawn_stage(
        &mut self,
        level: LevelRef,
        stage: DragonRespawnStage,
    ) {
        self.fire_dragon_respawn_stage(level, stage);
    }
    /// An End crystal of the fight was destroyed, after the fight answered it.
    /// Server-side.
    pub(in crate::game) fn native_end_crystal_destroyed(
        &mut self,
        level: LevelRef,
        pos: BlockPos,
        respawn_crystal: bool,
    ) {
        self.fire_end_crystal_destroyed(level, pos, respawn_crystal);
    }
    /// The End's dragon fight was won: whether to place an egg at `egg_pos`.
    /// Server-side, read-only.
    pub(in crate::game) fn native_dragon_fight_won(
        &self,
        level: LevelRef,
        egg_pos: BlockPos,
        previously_won: bool,
        place_egg: &mut Option<bool>,
    ) {
        self.fire_dragon_fight_won(level, egg_pos, previously_won, place_egg);
    }
}
