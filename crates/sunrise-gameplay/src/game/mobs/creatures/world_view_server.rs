use super::*;
use mod_api::{BlockEntityRef, Compound};
use sunrise_gameplay_common::game::creatures::world_view::*;
use sunrise_gameplay_common::game::side_ops::{
    modapi_modding::ModapiModdingSideGameExt as _, world::WorldSideGameExt as _,
};
use sunrise_items::items::Items;
use sunrise_level::world::World;
use sunrise_sim::motion::SideLevel;
use sunrise_world::registry::BlockRegistry;

/// The game's own side, for a tick that only reads it: each query builds the borrowed tables.
impl SideLevel for ServerGame<'_> {
    fn is_raining_at(&self, world: &World, pos: BlockPos) -> bool {
        sunrise_gameplay_common::game_side!(self, self.side_game_side_meta())
            .is_raining_at(world, pos)
    }

    fn enabled_features(&self) -> &std::sync::Arc<[String]> {
        sunrise_gameplay_common::game_side!(self, self.side_game_side_meta()).features
    }

    fn tag_items(&self) -> Option<&Items> {
        sunrise_gameplay_common::game_side!(self, self.side_game_side_meta()).tag_items
    }

    fn player_equipment(&self) -> sunrise_sim::health::PlayerEquipment<'_> {
        sunrise_gameplay_common::acting_equipment!(self)
    }

    fn living_target_alive(&self, id: u32) -> bool {
        self.side_living_target_alive(id)
    }

    fn hit_reads(&self) -> Option<sunrise_sim::health::HitReads<'_>> {
        crate::game::entity_server::hit_reads_of!(self)
    }

    fn damage_types(&self) -> &sunrise_world::damage_source::Catalog {
        &self.common.health_resources.damage
    }

    fn block_entity(&self, world: &World, pos: BlockPos) -> Option<BlockEntityRef> {
        sunrise_gameplay_common::game_side!(self, self.side_game_side_meta())
            .block_entity(world, pos)
    }

    fn block_entity_data(
        &self,
        world: &World,
        registry: &BlockRegistry,
        entity: BlockEntityRef,
    ) -> Option<Compound> {
        sunrise_gameplay_common::game_side!(self, self.side_game_side_meta())
            .block_entity_data(world, registry, entity)
    }

    fn world_epoch(&self) -> Option<mod_api::WorldEpoch> {
        self.common.native_lifecycle.epoch()
    }

    fn block_states(&self) -> Option<mod_api::BlockStates<'_>> {
        Some(mod_api::BlockStates::of(self))
    }

    fn variant_tables(&self) -> Option<&sunrise_sim::mobs::variants::VariantTables> {
        Some(self.world_variants())
    }
}

impl crate::game::ServerGame<'_> {
    pub fn mob_world_base<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s> {
        self.common.world.enable_path_type_cache();
        self.mob_world_base_with(
            others,
            MobWorldSideParams {
                side: self,
                trading_players: Some(self),
                player: self.player_view(),
                player_identity: self.player_identity(),
                leash_boats: &self.server.leash_boats,
                teams: std::sync::Arc::new(self.mob_teams()),
                lid_openers: &self.server.lid_openers,
                mod_effects: Some(self),
                mob_hooks: (self.native_authoritative()
                    && ([
                        mod_api::HookId::EndermanAnger,
                        mod_api::HookId::GazeDisguise,
                        mod_api::HookId::EntityMobGriefing,
                        mod_api::HookId::LivingVisibility,
                        mod_api::HookId::LivingChangeTarget,
                    ]
                    .into_iter()
                    .any(mod_api::__private::listened)
                        || [
                            mod_api::HookId::CanEntityDestroy,
                            mod_api::HookId::BlockPathType,
                            mod_api::HookId::AdjacentBlockPathType,
                        ]
                        .into_iter()
                        .any(mod_api::__private::has_scoped_hook)))
                .then_some(self as &dyn sunrise_sim::mobs::MobHooks),
                goal_host: (self.native_authoritative()
                    && ([
                        mod_api::HookId::RegisterGoals,
                        mod_api::HookId::RegisterBehaviors,
                    ]
                    .into_iter()
                    .any(mod_api::__private::has_scoped_hook)
                        || mod_api::__private::listened(mod_api::HookId::LivingChangeTarget)))
                .then_some(self as &dyn sunrise_sim::mobs::part_goals::GoalHost),
                asks_animal_tame: mod_api::__private::listened(mod_api::HookId::AnimalTame),
                asks_living_destroy_block: mod_api::__private::listened(
                    mod_api::HookId::LivingDestroyBlock,
                ),
                asks_baby_spawn: mod_api::__private::listened(mod_api::HookId::BabyEntitySpawn),
                // Server owns entity UUIDs so warden away suspects resolve here.
                who_lookup: Some(self),
                // First player in the level: a server has no player of its own.
                difficulty: self.common.local_difficulty(
                    self.level_players_iter()
                        .next()
                        .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]),
                ),
            },
        )
    }
}
