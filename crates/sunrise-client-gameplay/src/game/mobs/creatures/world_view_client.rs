use super::view_vector;
use mod_api::{BlockEntityRef, Compound};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use sunrise_client::actions::Action;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::creatures::world_view::MobWorldSideParams;
use sunrise_gameplay_common::game::native_block_tables::BlockTables;
use sunrise_gameplay_common::game::side_ops::{
    modapi_modding::ModapiModdingSideGameExt as _, world::WorldSideGameExt as _,
};
use sunrise_items::items::Items;
use sunrise_level::world::World;
use sunrise_sim::mobs::{MobWorld, PlayerView, Seen, Species, Vehicle};
use sunrise_sim::motion::SideLevel;
use sunrise_world::registry::BlockRegistry;

static EMPTY_LID_OPENERS: OnceLock<HashMap<(i32, i32, i32), i32>> = OnceLock::new();

impl SideLevel for crate::game::ClientGame<'_> {
    fn is_raining_at(&self, world: &World, pos: BlockPos) -> bool {
        client_game_side!(self, self.side_game_side_meta()).is_raining_at(world, pos)
    }

    fn enabled_features(&self) -> &std::sync::Arc<[String]> {
        client_game_side!(self, self.side_game_side_meta()).features
    }

    fn tag_items(&self) -> Option<&Items> {
        client_game_side!(self, self.side_game_side_meta()).tag_items
    }

    fn player_equipment(&self) -> sunrise_sim::health::PlayerEquipment<'_> {
        sunrise_gameplay_common::acting_equipment!(self)
    }

    fn living_target_alive(&self, id: u32) -> bool {
        self.side_living_target_alive(id)
    }

    fn damage_types(&self) -> &sunrise_world::damage_source::Catalog {
        &self.common.health_resources.damage
    }

    fn block_entity(&self, world: &World, pos: BlockPos) -> Option<BlockEntityRef> {
        client_game_side!(self, self.side_game_side_meta()).block_entity(world, pos)
    }

    fn block_entity_data(
        &self,
        world: &World,
        registry: &BlockRegistry,
        entity: BlockEntityRef,
    ) -> Option<Compound> {
        client_game_side!(self, self.side_game_side_meta())
            .block_entity_data(world, registry, entity)
    }

    fn world_epoch(&self) -> Option<mod_api::WorldEpoch> {
        self.common.native_lifecycle.epoch()
    }

    fn block_states(&self) -> Option<mod_api::BlockStates<'_>> {
        Some(mod_api::BlockStates::of(self))
    }
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn client_player_view(&self) -> Option<PlayerView> {
        let p = self.common.player.as_ref()?;
        let w = f64::from(p.dimensions.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        );
        let gold_tag = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:piglin_safe_armor");
        let gold_armor = self
            .common
            .armor
            .iter()
            .rev()
            .flatten()
            .any(|s| gold_tag.is_some_and(|t| t.contains(&*s.item)));
        let vehicle = self.common.riding.and_then(|id| {
            if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
                let w = 0.6875;
                return Some(Vehicle {
                    nautilus: false,
                    bounds: Aabb::new(
                        Vec3::new(b.position.x - w, b.position.y, b.position.z - w),
                        Vec3::new(b.position.x + w, b.position.y + 0.5625, b.position.z + w),
                    ),
                    velocity: b.velocity,
                });
            }
            let m = self.common.mobs.iter().find(|m| m.id == id)?;
            matches!(m.species(), Species::Nautilus | Species::ZombieNautilus).then(|| Vehicle {
                nautilus: true,
                bounds: m.bounds(),
                velocity: m.velocity,
            })
        });
        Some(PlayerView {
            position: p.position,
            bounds,
            eye: [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ],
            alive: !p.health.dead_or_dying(),
            invulnerable: p.invulnerable
                || p.health.permanently_invulnerable
                || p.health.invulnerable_time > 0,
            creative: self.common.creative || self.common.spectator(),
            sneaking: self.held_mapping(Action::Sneak),
            swimming: p.swimming,
            look: view_vector(self.common.yaw, self.common.pitch),
            main_hand: self.common.held().map(|s| s.item.clone()),
            off_hand: self.common.offhand.as_ref().map(|s| s.item.clone()),
            head: self.common.armor[0].as_ref().map(|s| s.item.clone()),
            chest: self.common.armor[1].as_ref().map(|s| s.item.clone()),
            legs: self.common.armor[2].as_ref().map(|s| s.item.clone()),
            feet: self.common.armor[3].as_ref().map(|s| s.item.clone()),
            armor_cover: self.common.armor.iter().filter(|a| a.is_some()).count() as f32 / 4.0,
            invisible: self.common.potions.effects.get("invisibility").is_some(),
            spectator: self.common.spectator(),
            gold_armor,
            hurt_by: None,
            last_hurt: p.health.last_hurt_mob,
            velocity: p.velocity,
            yaw: self.common.yaw,
            sprinting: p.sprinting,
            riding: self.common.riding.is_some(),
            hero: self
                .common
                .potions
                .effects
                .get("hero_of_the_village")
                .is_some(),
            effects: self
                .common
                .potions
                .effects
                .list
                .iter()
                .map(|e| e.id)
                .collect(),
            uuid: self.common.level.player_uuid.unwrap_or([0; 4]),
            allies: Arc::new(HashSet::new()),
            remote: true,
            sleeping: self.common.sleeping.is_some(),
            sleep_timer: self.common.sleep_timer,
            passenger: self.common.riding.is_some(),
            riding_vehicle: self.common.riding,
            vehicle,
        })
    }

    pub(in crate::game) fn client_mob_side(
        &self,
    ) -> sunrise_gameplay_common::game::native_surface::GameSide<'_> {
        use sunrise_items::equipment::{BODY, SADDLE};
        let level = self.common.native_lifecycle.epoch().and_then(|epoch| {
            let key = self.common.dim.key();
            mod_api::__private::dimension_id(key)?;
            Some(mod_api::LevelRef::with_generation(epoch, key, 0))
        });
        sunrise_gameplay_common::game::native_surface::GameSide {
            dim: self.common.dim,
            level,
            facts: self.common.res.facts(),
            tables: BlockTables::new(&self.common.res, &self.common.pack),
            biomes: match &self.common.world_biomes {
                Some(b) => b,
                None => self.common.res.biomes(),
            },
            behaviours: self.common.res.behaviours(),
            features: match &self.client.remote {
                Some(remote) => &remote.features,
                None => sunrise_gameplay_common::game::tables::default_client_features(),
            },
            items: self.common.res.items(),
            damage_types: &self.common.health_resources.damage,
            player_equipment: [
                self.common.inventory[self.common.selected].as_ref(),
                self.common.offhand.as_ref(),
                self.common.armor[3].as_ref(),
                self.common.armor[2].as_ref(),
                self.common.armor[1].as_ref(),
                self.common.armor[0].as_ref(),
                self.common.equipment.get(BODY),
                self.common.equipment.get(SADDLE),
            ],
            hit_borrows: sunrise_gameplay_common::game::native_surface::HitBorrows {
                living_types: self.common.res.living_type_keys(),
                attributes: &self.common.motion.attributes,
                player_effects: &self.common.potions.effects,
                player_uuid: None,
                player_attachments: &self.common.player_state.attachments,
                entity_attachments: &self.common.entity_attachments,
            },
            tag_items: self
                .client
                .remote
                .as_ref()
                .and_then(|remote| remote.tags.as_ref()),
        }
    }

    pub(in crate::game) fn client_mob_world_base<'s>(
        &'s self,
        others: &'s [Seen],
        side: &'s dyn sunrise_sim::motion::SideLevel,
    ) -> MobWorld<'s> {
        let at = self
            .common
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        self.mob_world_base_with(
            others,
            MobWorldSideParams {
                side,
                trading_players: None,
                player: self.client_player_view(),
                player_identity: None,
                leash_boats: &[],
                teams: Arc::new(self.client_mob_teams()),
                lid_openers: EMPTY_LID_OPENERS.get_or_init(HashMap::new),
                mod_effects: None,
                mob_hooks: None,
                goal_host: None,
                asks_animal_tame: false,
                asks_living_destroy_block: false,
                asks_baby_spawn: false,
                who_lookup: None,
                difficulty: self.common.local_difficulty(at),
            },
        )
    }
}

use sunrise_gameplay_common::game::creatures::world_view::WorldViewExt as _;
