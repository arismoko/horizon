//! A level's world settings through a `LevelRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): the seed,
//! difficulty and game rules on the server, and weather on both sides.
//! Stale handles return `None`.
use super::Dim;
use core::ops::ControlFlow;
use mod_api::{BoolGameRuleKey, FeatureFlags, LevelReadError, LevelRef};
use sunrise_command::vanilla::DIFFICULTIES;
use sunrise_core::position::BlockPos;
use sunrise_game_data::gamerules::Value as GameRuleValue;

use sunrise_gameplay_common::game::native_rain_read::{rain_at, weather_flags, weather_for_world};

impl crate::game::ServerGame<'_> {
    /// The sea level the game uses for the level (`Dim::sea_level`). None as the host call says.
    pub(in crate::game) fn level_sea_level(&self, level: LevelRef) -> Option<i32> {
        self.native_level_world(level)?;
        Some(Dim::sea_level_of_id(level.dimension_id()))
    }

    /// The world's seed (`WorldOptions.seed`), as `/seed` shows it.
    pub(in crate::game) fn level_seed(&self, level: LevelRef) -> Option<i64> {
        self.native_level_world(level)?;
        Some(self.common.level.seed)
    }

    /// The codec ID of the generator the server built for the named level.
    pub(in crate::game) fn level_generator_type(
        &self,
        level: LevelRef,
    ) -> Option<mod_api::Identifier> {
        self.native_level_world(level)?.generator_type.clone()
    }

    /// The difficulty (`Level.getDifficulty`): `peaceful`, `easy`, `normal`
    /// or `hard`, as `/difficulty` names it.
    pub(in crate::game) fn level_difficulty(&self, level: LevelRef) -> Option<&'static str> {
        self.native_level_world(level)?;
        DIFFICULTIES
            .get(usize::from(self.common.ui.difficulty))
            .copied()
    }
    /// `ServerLevel.getCurrentDifficultyAt(pos)` in the level the handle names, in place or
    /// parked.
    pub(in crate::game) fn level_local_difficulty(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<mod_api::Difficulty> {
        let world = self.native_level_world(level)?;
        let at = pos.map(f64::from);
        Some(sunrise_gameplay_common::game::creatures::local_difficulty_in(world, at).into())
    }
    /// Read a registered boolean game rule.
    pub(in crate::game) fn level_game_rule_bool(
        &self,
        level: LevelRef,
        key: BoolGameRuleKey,
    ) -> Option<bool> {
        self.native_level_world(level)?;
        self.common
            .ui
            .rules
            .bool_slot(mod_api::__private::game_rule_bool_slot(key)?)
    }
    /// Read a registered integer game rule.
    pub(in crate::game) fn level_game_rule_int(
        &self,
        level: LevelRef,
        key: mod_api::IntGameRuleKey,
    ) -> Option<i32> {
        self.native_level_world(level)?;
        self.common
            .ui
            .rules
            .int_slot(mod_api::__private::game_rule_int_slot(key)?)
    }
    /// Game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`), as
    /// `/gamerule` queries it: None for an unknown rule.
    pub(in crate::game) fn level_game_rule(
        &self,
        level: LevelRef,
        rule: &str,
    ) -> Result<Option<GameRuleValue>, LevelReadError> {
        self.native_level_world_result(level)?;
        Ok(self.common.ui.rules.get(rule))
    }
    /// `/gamerule <rule> <value>` from a mod: the world's rule changes, `game_rule_changed`
    /// fires with no actor and the clients get the rules. The rule already at `value` is true
    /// with nothing fired; an unknown rule, a value of the other kind or out of range is false.
    pub fn level_set_game_rule(
        &mut self,
        level: LevelRef,
        rule: &str,
        value: GameRuleValue,
    ) -> bool {
        if self.native_level_world(level).is_none() {
            return false;
        }
        let Some(rule) = self.common.ui.rules.rule(rule).cloned() else {
            return false;
        };
        let fits = match (rule.default, value) {
            (GameRuleValue::Bool(_), GameRuleValue::Bool(_)) => true,
            (GameRuleValue::Int(_), GameRuleValue::Int(v)) => (rule.min..=rule.max).contains(&v),
            _ => false,
        };
        if fits && self.common.ui.rules.get(&rule.name) != Some(value) {
            self.set_game_rule(&rule, value, None);
        }
        fits
    }
    /// `/time set <time>` in the level a handle names, on its default clock: `time_skip`
    /// (cause `mod`) may keep the old time; the clock's partial tick restarts.
    pub fn level_set_clock_time(&mut self, level: LevelRef, time: i64) -> bool {
        if time < 0 || self.native_level_world(level).is_none() {
            return false;
        }
        let dim = Dim::from_id(level.dimension_id());
        if self.common.dimension_type_of(dim).default_clock.is_none() {
            return false;
        }
        let clock = self.common.level_clock_name(dim);
        let total = self.common.command_clock_time(&clock);
        if total == time {
            return true;
        }
        let mut cancel = false;
        self.native_time_skip(level.dimension_id(), total, time, "mod", &mut cancel);
        if cancel {
            return false;
        }
        self.common.set_command_clock_time(&clock, time);
        self.common.command_clock_state(&clock).partial = 0.0;
        true
    }
    /// `DimensionType.defaultClock` of a level the server has open, as a handle.
    pub(in crate::game) fn level_clock(&self, level: LevelRef) -> Option<mod_api::WorldClockRef> {
        self.world_clock_name(mod_api::WorldClockRef::new(level))
            .map(|_| mod_api::WorldClockRef::new(level))
    }
    /// The `minecraft:world_clock` id a clock handle names now: its level's default clock.
    fn world_clock_name(&self, clock: mod_api::WorldClockRef) -> Option<String> {
        let level = clock.level();
        self.native_level_world(level)?;
        let dim = Dim::from_id(level.dimension_id());
        self.common.dimension_type_of(dim).default_clock.clone()
    }
    /// `ServerClockManager.setTotalTicks`: the total, its fraction of a tick restarted.
    pub(in crate::game) fn world_clock_set_total_ticks(
        &mut self,
        clock: mod_api::WorldClockRef,
        ticks: i64,
    ) -> bool {
        let Some(name) = self.world_clock_name(clock).filter(|_| ticks >= 0) else {
            return false;
        };
        self.common.set_command_clock_time(&name, ticks);
        self.common.command_clock_state(&name).partial = 0.0;
        self.sync_hub_clocks();
        true
    }
    /// `ServerClockManager.setRate`.
    pub(in crate::game) fn world_clock_set_rate(
        &mut self,
        clock: mod_api::WorldClockRef,
        rate: f32,
    ) -> bool {
        let Some(name) = self
            .world_clock_name(clock)
            .filter(|_| rate.is_finite() && rate >= 0.0)
        else {
            return false;
        };
        self.common.command_clock_state(&name).rate = rate;
        self.sync_hub_clocks();
        true
    }
    /// `ServerClockManager.setPaused`.
    pub(in crate::game) fn world_clock_set_paused(
        &mut self,
        clock: mod_api::WorldClockRef,
        paused: bool,
    ) -> bool {
        let Some(name) = self.world_clock_name(clock) else {
            return false;
        };
        self.common.command_clock_state(&name).paused = paused;
        self.sync_hub_clocks();
        true
    }
    /// Visit the server's available game rules once the named level resolves.
    pub(in crate::game) fn level_visit_game_rules(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(&str, mod_api::GameRuleValue) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        self.native_level_world_result(level)?;
        Ok(self.common.visit_available_game_rules(visit))
    }
    /// Whether it rains (`Level.isRaining`: a rain level above 0.2).
    pub(in crate::game) fn level_is_raining(&self, level: LevelRef) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(
            weather_flags(weather_for_world(
                world,
                level.dimension() == Dim::End.key(),
            ))
            .0,
        )
    }
    /// `Level.isRainingAt(pos)` in the level a handle names, the one in place or a parked one:
    /// it rains there and `pos` is under open sky in a biome that rains. None for a stale
    /// handle, on a client, and where `pos` is not loaded or outside the world's height; a
    /// read never loads a chunk.
    pub fn level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        let world = self.native_level_world(level)?;
        if !world.is_loaded_at(pos[0], pos[1], pos[2]) {
            return None;
        }
        self.level_is_raining(level)?;
        Some(rain_at(
            world,
            self.common.res.facts(),
            world_biomes!(self),
            self.common.res.behaviours(),
            Dim::from_id(level.dimension_id()),
            BlockPos::new(pos[0], pos[1], pos[2]),
        ))
    }
    /// `LevelReader.enabledFeatures` of the level a handle names. Every level of a world shares
    /// the world's flags. None for a stale handle and on a client.
    pub fn level_enabled_features(&self, level: LevelRef) -> Option<FeatureFlags> {
        self.native_level_world(level)?;
        Some(FeatureFlags::new(self.enabled_feature_ids().clone()))
    }
    /// Whether it thunders (`Level.isThundering`: a thunder level above 0.9).
    pub(in crate::game) fn level_is_thundering(&self, level: LevelRef) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(
            weather_flags(weather_for_world(
                world,
                level.dimension() == Dim::End.key(),
            ))
            .1,
        )
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::clocks::ClocksGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
