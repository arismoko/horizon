//! Events, explosions and lightning in a level for native server mods,
//! through a `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"):
//! `Level.levelEvent`, `Level.gameEvent`, `ServerLevel.blockEvent`,
//! `Level.explode` and a summoned lightning bolt. Like the level's block
//! writes, they act on the level in place, the one the hook runs in; a parked
//! level, a stale handle or a client returns false. Reaching a parked level, as `play_sound` and the
//! reads do through `native_level_world`, is a follow-up.
use crate::game::ServerGame;
use mod_api::{
    BlockKey, EntityRef, ExplodeError, ExplosionInteraction, ExplosionSettings, LevelRef,
};
use sunrise_gameplay_common::game::mod_api_host::level_event_data_ok;
use sunrise_world::{level::BlockEventData, registry::BlockStateId};

/// A position inside the world's horizontal and vertical bounds.
fn in_bounds(at: [f64; 3]) -> bool {
    at.iter().all(|v| v.is_finite())
        && at[0].abs() < 30_000_000.0
        && at[2].abs() < 30_000_000.0
        && at[1].abs() < 20_000_000.0
}

impl ServerGame<'_> {
    /// Plays level event `id` at `pos` with `data` for the players near it
    /// (`Level.levelEvent(null, id, pos, data)`): `1000`s are sounds, `2000`s
    /// particles, as vanilla's `LevelEvent` numbers them (`2001` breaks a
    /// block with state id `data`). No player is left out, the acting one
    /// included. False for an event that carries a block state (2001, 2014,
    /// 3008) when `data` isn't one.
    pub fn level_level_event(
        &mut self,
        level: LevelRef,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool {
        if !self.level_in_place(level) {
            return false;
        }
        if !level_event_data_ok(id, data, self.common.res.facts().states.len()) {
            return false;
        }
        self.emit_level_effect(
            sunrise_gameplay_common::game::journal_level_effects::PendingWrite::Event {
                level: super::Dim::from_id(level.dimension_id()),
                id,
                pos,
                data,
            },
        );
        true
    }
    /// Plays the break effect of `state` at `pos` (`Level.levelEvent(2001, pos,
    /// Block.getId(state))`); the state's id is resolved here, never by the mod.
    pub fn level_block_break_effect(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> bool {
        self.level_level_event(level, 2001, pos, state.0 as i32)
    }
    /// Plays level event `id` for every player on the server
    /// (`ServerLevel.globalLevelEvent`), or for the players near `pos` when the
    /// `global_sound_events` rule is off. Refuses what `level_level_event` refuses.
    pub fn level_global_level_event(
        &mut self,
        level: LevelRef,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool {
        if !self.level_in_place(level) {
            return false;
        }
        if !level_event_data_ok(id, data, self.common.res.facts().states.len()) {
            return false;
        }
        self.emit_level_effect(
            sunrise_gameplay_common::game::journal_level_effects::PendingWrite::GlobalEvent {
                level: super::Dim::from_id(level.dimension_id()),
                id,
                pos,
                data,
            },
        );
        true
    }
    /// Sends game event `event` (`minecraft:block_change`...) from `at`, as
    /// `Level.gameEvent(event, at, context)`, with validated source and state.
    pub fn level_game_event(
        &mut self,
        level: LevelRef,
        event: &str,
        at: [f64; 3],
        context: mod_api::GameEventContext,
    ) -> bool {
        let Some(event) = sunrise_level::sculk::intern(event) else {
            return false;
        };
        if !self.level_in_place(level) || !in_bounds(at) {
            return false;
        }
        self.post_contextual_game_event(event, at, context)
    }
    /// Queues block event `a`, `b` for `block` at `pos` in the level in place
    /// (`ServerLevel.blockEvent`): `run_block_events` hands it to the block's
    /// `triggerEvent` after this tick's scheduled ticks if `pos` still holds
    /// `block`. False for an unresolved block key, a position outside the
    /// build height (it could never reach a block), a parked level, a stale
    /// handle or a client.
    pub(in crate::game) fn level_block_event(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        block: BlockKey,
        a: i32,
        b: i32,
    ) -> bool {
        let Some(block) = sunrise_gameplay_common::game::api_values::behavior_block(
            block,
            self.common.res.behaviours(),
        ) else {
            return false;
        };
        if !self.level_in_place(level) || !self.common.world.shape.contains_y(pos[1]) {
            return false;
        }
        self.common.block_ticks.queue_block_event(BlockEventData {
            pos: sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            block,
            param_a: a,
            param_b: b,
            note_override: None,
        });
        true
    }
    /// Explodes at `center` as vanilla's full `Level.explode(source,
    /// damageSource, calculator, x, y, z, radius, fire, interaction, particles...,
    /// sound)` does (`native_explode`), the causing entity derived from
    /// `source`. Refused, with nothing exploded, for another level, a stale
    /// `source` or one in another level, once the entity ids are exhausted, and as
    /// `native_explode` refuses.
    pub fn level_explode(
        &mut self,
        level: LevelRef,
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
        settings: ExplosionSettings,
    ) -> Result<(), ExplodeError> {
        if self.entity_ids_exhausted() {
            return Err(ExplodeError::Unavailable);
        }
        if !self.level_in_place(level) {
            return Err(ExplodeError::Unavailable);
        }
        let source = match source {
            Some(entity) => Some(
                self.native_explosion_source(entity)
                    .ok_or(ExplodeError::Unavailable)?,
            ),
            None => None,
        };
        self.native_explode(source, center, radius, fire, interaction, settings)
    }
    /// Strikes lightning at `at`, as summoning a `minecraft:lightning_bolt`
    /// does: it sets fire, hurts and converts what it hits (a pig to a
    /// zombified piglin...) and every player hears the thunder. With
    /// `visual_only` it only flashes and sounds (`setVisualOnly`). The
    /// `lightning_strike` hook hears it first with the cause `mod` and may
    /// cancel it. False for a chunk that isn't loaded or a canceled strike.
    pub(in crate::game) fn level_strike_lightning(
        &mut self,
        level: LevelRef,
        at: [f64; 3],
        visual_only: bool,
    ) -> bool {
        if !self.level_in_place(level) || !in_bounds(at) {
            return false;
        }
        let chunk = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
        if !self.common.world.chunks.contains_key(&chunk) {
            return false;
        }
        let block = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        if !self.lightning_strike_allowed(None, block, "mod") {
            return false;
        }
        self.spawn_bolt(at, visual_only);
        true
    }
}

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
