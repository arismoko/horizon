//! The immutable world view used by mob simulation.
use super::*;
use crate::game::GameGameExt1 as _;

/// Side-specific inputs supplied to [`Game::mob_world_base_with`] by server or client.
pub struct MobWorldSideParams<'s> {
    pub side: &'s dyn sunrise_sim::motion::SideLevel,
    pub trading_players: Option<&'s dyn sunrise_sim::mobs::villager_brain::TradingPlayers>,
    pub player: Option<PlayerView>,
    pub player_identity: Option<([i32; 4], u32)>,
    pub leash_boats: &'s [(u32, sunrise_sim::mobs::leash::HolderState)],
    pub teams: std::sync::Arc<sunrise_sim::mobs::Teams<'s>>,
    pub lid_openers: &'s std::collections::HashMap<(i32, i32, i32), i32>,
    pub mod_effects: Option<&'s dyn sunrise_sim::mobs::ModEffectTicks>,
    pub mob_hooks: Option<&'s dyn sunrise_sim::mobs::MobHooks>,
    pub goal_host: Option<&'s dyn sunrise_sim::mobs::part_goals::GoalHost>,
    pub asks_animal_tame: bool,
    pub asks_living_destroy_block: bool,
    pub asks_baby_spawn: bool,
    pub who_lookup: Option<&'s dyn sunrise_sim::mobs::WhoLookup>,
    pub difficulty: sunrise_sim::mobs::spawning::LocalDifficulty,
}

pub trait WorldViewExt<'__sg>: crate::game::SideGame<'__sg> {
    fn mob_world_base_with<'s>(
        &'s self,
        others: &'s [Seen],
        params: MobWorldSideParams<'s>,
    ) -> MobWorld<'s>
    where
        '__sg: 's,
    {
        let (rain, thunder) = self.weather_levels(1.0);
        let weather = self.can_have_weather();
        let nearest_player = params
            .player
            .as_ref()
            .filter(|p| !p.spectator)
            .map(|p| p.position);
        MobWorld {
            side: params.side,
            mob_maps: self.mob_maps(),
            section_writer: None,
            trading_players: params.trading_players,
            professions: self.stations().professions.as_ref(),
            loop_context: &self.loop_context,
            world: &self.world,
            registry: self.pack.assets.registry,
            facts: self.res.facts(),
            fluid_tables: &self.motion.fluids,
            attributes: &self.motion.attributes,
            movement: &self.motion.blocks,
            collision: self.collision,
            suffocation: self.level_suffocation(),
            data: self.res.mob_data(),
            controlling_player_movement: None,
            nearest_player,
            player: params.player,
            player_identity: params.player_identity,
            player_selection: None,
            others,
            live_mobs: None,
            dragon: self.dragon_target(),
            armor_stands: &self.armor_stands,
            leash_boats: params.leash_boats,
            others_grid: None,
            others_index: None,
            passenger_objects: &[],
            others_villagers: None,
            others_solid: None,
            daylight: self.dim != Dim::Nether && self.sky_darken() < 4,
            // day.json: false from tick 12542, true from 23460 (a step track).
            monsters_burn: self.dimension_type().has_timeline("minecraft:day")
                && !(12542..23460).contains(&self.world.day_time.rem_euclid(24000)),
            game_time: self.world.game_time,
            day_time: self.world.day_time,
            raining: weather && rain > 0.2,
            thundering: weather && thunder > 0.9,
            level: self.level_key(),
            block_tags: &self.items().block_tags,
            block_tag_flags: self.items().block_tag_flags.bits(),
            landing: &self.motion.landing,
            biomes: world_biomes!(self),
            mob_griefing: self
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING),
            universal_anger: self
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::UNIVERSAL_ANGER),
            mob_drops: self
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::MOB_DROPS),
            pvp: self
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::PVP),
            max_entity_cramming: self
                .ui
                .rules
                .int_at(sunrise_game_data::gamerules::vanilla::MAX_ENTITY_CRAMMING),
            dimension_type: self.dimension_type(),
            world_seed: self.level.seed,
            sea_level: self.dim.sea_level(),
            teams: params.teams,
            items: Some(self.items()),
            lid_openers: params.lid_openers,
            mod_effects: params.mod_effects,
            mob_hooks: params.mob_hooks,
            goal_host: params.goal_host,
            asks_animal_tame: params.asks_animal_tame,
            asks_living_destroy_block: params.asks_living_destroy_block,
            asks_baby_spawn: params.asks_baby_spawn,
            who_lookup: params.who_lookup,
            border: Some(self.border.geometry()),
            difficulty: params.difficulty,
        }
    }

    /// `Level.getEntity`: revenge goals must resolve the dragon too.
    fn dragon_target(&self) -> Option<(u32, sunrise_sim::mobs::TargetInfo)> {
        let d = self.dragon.as_ref().filter(|d| d.alive())?;
        let p = d.position;
        let bounds = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        );
        Some((
            d.id,
            sunrise_sim::mobs::TargetInfo {
                position: p,
                eye_y: p.y + 6.8,
                bounds,
                hitbox: bounds,
                alive: true,
                can_be_seen_as_enemy: true,
                is_player: false,
                creative: false,
                passenger: false,
            },
        ))
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> WorldViewExt<'__sg> for T {}

use crate::game::tables::GameItems as _;

use crate::game::tables::WorldTablesExt as _;

use crate::game::weather::WeatherGameExt as _;
