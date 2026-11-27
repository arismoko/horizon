//! Client-side entity state reads and client fire-immunity writes.
use super::commands_common::Target;
use mod_api::{EntityRef, EntityTypeKey, MobCategory};
use sunrise_gameplay_common::game::entity_sizes_common::synced_pose;
use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;
use sunrise_gameplay_common::game::general::general_type_catalog;
use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_base::BuiltinKind;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_state_common::{
    Body, NativeEntityStateCommonGameExt1 as _,
};
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
use sunrise_sim::pose::Pose;

/// `Pose`'s serialized names by id (`Pose.STANDING` is 0).
const POSES: [&str; 18] = [
    "standing",
    "fall_flying",
    "sleeping",
    "swimming",
    "spin_attack",
    "crouching",
    "long_jumping",
    "dying",
    "croaking",
    "using_tongue",
    "sitting",
    "roaring",
    "sniffing",
    "emerging",
    "digging",
    "sliding",
    "shooting",
    "inhaling",
];

/// A player's pose as `Pose` names it.
fn player_pose(pose: Pose) -> &'static str {
    match pose {
        Pose::Standing => "standing",
        Pose::Crouching => "crouching",
        Pose::Swimming => "swimming",
        Pose::FallFlying => "fall_flying",
        Pose::SpinAttack => "spin_attack",
        Pose::Sleeping => "sleeping",
    }
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn client_entity_body_fire_facts(
        &self,
        entity: EntityRef,
        target: Target,
    ) -> Option<(i32, bool, Option<BuiltinKind>)> {
        use sunrise_sim::projectiles::ProjectileKind;

        let common = &self.common;
        let (ticks, immune, kind) = match target {
            Target::Player => {
                let player = common.player.as_ref()?;
                (player.fire_ticks, player.mod_fire_immune, None)
            }
            Target::Guest(_) => return None,
            Target::Mob(id) => {
                let mob = common.mobs.iter().find(|mob| mob.id == id)?;
                (mob.fire_ticks, mob.fire_immune(), None)
            }
            Target::Stand(id) => {
                let stand = common.armor_stands.iter().find(|stand| stand.id == id)?;
                (stand.fire_ticks, stand.mod_fire_immune, None)
            }
            Target::Item(id) => {
                let item = common.dropped.iter().find(|item| item.id == id)?;
                let immune =
                    self.stack_can_be_hurt_by(&item.stack, "minecraft:in_fire") == Some(false);
                (item.fire_ticks, immune, None)
            }
            Target::Orb(id) => (
                common.orbs.iter().find(|orb| orb.id == id)?.fire_ticks,
                false,
                None,
            ),
            Target::Boat(id) => (
                common.boats.iter().find(|boat| boat.id == id)?.fire_ticks,
                false,
                None,
            ),
            Target::Projectile(id) => {
                let shot = common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|shot| shot.id == id)?;
                let kind = match shot.kind {
                    ProjectileKind::WitherSkull { .. } => Some(BuiltinKind::WitherSkull),
                    ProjectileKind::ShulkerBullet { .. } => Some(BuiltinKind::ShulkerBullet),
                    _ => None,
                };
                let ticks = shot
                    .arrow
                    .as_ref()
                    .map_or(shot.fire_ticks, |arrow| arrow.fire_ticks);
                (ticks, false, kind)
            }
            Target::Trident(id) => (
                common
                    .tridents
                    .iter()
                    .find(|trident| trident.id == id)?
                    .fire_ticks,
                false,
                None,
            ),
            Target::Falling(id) => (
                common
                    .falling
                    .iter()
                    .find(|block| block.id == id)?
                    .fire_ticks,
                false,
                None,
            ),
            Target::Crystal(id) => (
                common
                    .crystals
                    .iter()
                    .find(|crystal| crystal.id == id)?
                    .fire,
                false,
                Some(BuiltinKind::EndCrystal),
            ),
            Target::Bolt(id) => (
                common.bolts.iter().find(|bolt| bolt.id == id)?.fire_ticks,
                false,
                None,
            ),
            Target::Fangs(id) => (
                common
                    .fangs
                    .list
                    .iter()
                    .find(|fangs| fangs.id == id)?
                    .fire_ticks,
                false,
                None,
            ),
            Target::Dragon(_) | Target::DragonPart(_) => (0, false, Some(BuiltinKind::EnderDragon)),
            Target::Cloud(_) => (0, false, Some(BuiltinKind::AreaEffectCloud)),
            Target::Tnt(_) => (0, false, Some(BuiltinKind::Tnt)),
            Target::General(_) => {
                let row = self.native_client_general(entity)?;
                (0, general_type_catalog().fire_immune(row.type_slot), None)
            }
            _ => (0, false, None),
        };
        Some((
            ticks,
            immune || kind.is_some_and(BuiltinKind::fire_immune),
            kind,
        ))
    }

    pub(in crate::game) fn entity_type_category(&self, key: EntityTypeKey) -> Option<MobCategory> {
        let name = mod_api::__private::entity_type_key_id(key)?;
        Some(match &*name {
            "minecraft:ender_dragon" => MobCategory::Monster,
            name => self
                .common
                .res
                .mob_types()
                .get(name)
                .map_or(MobCategory::Misc, |kind| mob_category(&kind.category)),
        })
    }

    pub(in crate::game) fn entity_spectator(&self, entity: EntityRef) -> Option<bool> {
        Some(match self.native_entity_read_target(entity)? {
            Target::Player => self.own_player_is_spectator(),
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|mob| mob.id == id)?;
                &*mob.kind.id == "minecraft:player" && mob.extra.spectator
            }
            _ => false,
        })
    }

    pub(in crate::game) fn entity_is_sitting(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Mob(mob) => mob.sitting(),
            Body::Player(_) | Body::Other(_) => None,
        }
    }
    pub(in crate::game) fn entity_custom_name_visible(&self, entity: EntityRef) -> Option<bool> {
        self.native_client_general(entity)
            .map(|row| row.custom_name_visible)
    }

    /// `Entity.tickCount` of a client's own copy: the local player's, or that of an entity the
    /// server added to the level the client shows, whatever its kind, counted by the client's
    /// entity pass (`Entity.commonTick`) since the client added it.
    pub(in crate::game) fn client_entity_tick_count(&self, entity: EntityRef) -> Option<i32> {
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        // The local player from its admission (Login, Respawn), before the server's first
        // teleport gives it a body.
        if id == remote.entity_id {
            return remote.has_logical_player.then_some(remote.own_tick_count);
        }
        remote
            .entities
            .get(&id)
            .map(|tracked| tracked.tick_count)
            .or_else(|| remote.general.rows.get(&id).map(|row| row.age as i32))
    }

    pub(in crate::game) fn client_tracked_player_fire_immunity(
        &self,
        entity: EntityRef,
    ) -> Option<bool> {
        let (kind, _) = self.native_client_tracked(entity)?;
        (kind == "minecraft:player").then_some(())?;
        self.client
            .remote
            .as_ref()?
            .tracked_fire_immunity(i32::try_from(entity.id()).ok()?)
    }

    pub(in crate::game) fn client_entity_is_shift_key_down(
        &self,
        entity: EntityRef,
    ) -> Option<bool> {
        Some(match self.native_client_entity_target(entity)? {
            Target::Player => self.held_mapping(sunrise_client::actions::Action::Sneak),
            _ => false,
        })
    }

    pub(in crate::game) fn client_entity_main_arm(
        &self,
        entity: EntityRef,
    ) -> Option<mod_api::MainHand> {
        let target = self.native_client_entity_target(entity)?;
        let left = match target {
            Target::Player => self.client.settings.main_hand_left,
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .main_arm_left(),
            Target::Stand(_) | Target::Dragon(_) => false,
            _ => return None,
        };
        Some(if left {
            mod_api::MainHand::Left
        } else {
            mod_api::MainHand::Right
        })
    }

    pub(in crate::game) fn entity_set_client_fire_immunity(
        &mut self,
        entity: EntityRef,
        immune: bool,
    ) -> bool {
        if let Some(target) = self.native_client_entity_target(entity) {
            let (grant, ticks) = match target {
                Target::Mob(id) => {
                    let Some(mob) = self
                        .common
                        .mobs
                        .iter_mut()
                        .find(|m| m.id == id && !m.removed)
                    else {
                        return false;
                    };
                    (&mut mob.mod_fire_immune, &mut mob.fire_ticks)
                }
                Target::Stand(id) => {
                    let Some(stand) = self.common.armor_stands.iter_mut().find(|s| s.id == id)
                    else {
                        return false;
                    };
                    (&mut stand.mod_fire_immune, &mut stand.fire_ticks)
                }
                Target::Player => {
                    let Some(player) = self.common.player.as_mut() else {
                        return false;
                    };
                    (&mut player.mod_fire_immune, &mut player.fire_ticks)
                }
                _ => return false,
            };
            *grant = immune;
            if immune && matches!(target, Target::Player) {
                *ticks = (*ticks).min(0);
            }
            return true;
        }
        let tracked_player = self
            .native_client_tracked(entity)
            .is_some_and(|(kind, _)| kind == "minecraft:player")
            && self
                .client
                .remote
                .as_ref()
                .and_then(|r| {
                    i32::try_from(entity.id())
                        .ok()
                        .and_then(|id| r.tracked_fire_immunity(id))
                })
                .is_some();
        if !tracked_player {
            return false;
        }
        self.client
            .remote
            .as_mut()
            .is_some_and(|remote| remote.set_tracked_fire_immunity(entity.id() as i32, immune))
    }

    /// The entity's pose (`Entity.getPose`) on a client: the local player's predicted pose, or
    /// the synched `DATA_POSE` the server last sent for a kept entity.
    pub(in crate::game) fn entity_pose(&self, entity: EntityRef) -> Option<&'static str> {
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(p) => player_pose(p.pose),
                Body::Mob(m) => POSES[synced_pose(m) as usize],
                Body::Other(_) => POSES[0],
            },
        )
    }

    pub(in crate::game) fn entity_is_crouching(&self, entity: EntityRef) -> Option<bool> {
        if self.native_entity_read_target(entity)? == Target::Player {
            return Some(self.common.player.as_ref()?.local_crouching);
        }
        Some(self.entity_pose(entity)? == "crouching")
    }

    pub(in crate::game) fn entity_is_sprinting(&self, entity: EntityRef) -> Option<bool> {
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(player) => player.sprinting,
                Body::Mob(mob) => mob.extra.sprinting,
                Body::Other(_) => false,
            },
        )
    }

    pub(in crate::game) fn entity_is_in_lava(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(player) => Some(player.is_in_lava()),
            Body::Mob(mob) => Some(mob.fluids.in_lava),
            Body::Other(Target::General(id)) => Some(
                self.common
                    .resident_entities
                    .general
                    .row(id)?
                    .fluids
                    .in_lava,
            ),
            Body::Other(_) => None,
        }
    }

    pub(in crate::game) fn entity_invulnerable_time(&self, entity: EntityRef) -> Option<i32> {
        match self.native_entity_read_target(entity)? {
            Target::Player => Some(self.common.player.as_ref()?.health.invulnerable_time),
            Target::Mob(id) => Some(
                self.common
                    .mobs
                    .iter()
                    .find(|m| m.id == id)?
                    .invulnerable_time,
            ),
            _ => None,
        }
    }

    pub(in crate::game) fn entity_damage_cooldown_time(&self, entity: EntityRef) -> Option<i32> {
        match self.native_entity_read_target(entity)? {
            Target::Player => Some(self.common.player.as_ref()?.health.hurt.cooldown),
            Target::Mob(id) => Some(self.common.mobs.iter().find(|m| m.id == id)?.invulnerable),
            _ => None,
        }
    }

    /// `LivingEntity.yBodyRot` on a client: the local player's presentation model yaw.
    pub(in crate::game) fn entity_body_yaw(&self, entity: EntityRef) -> Option<f32> {
        self.native_client_level()?;
        if self.local_player()?.entity() == entity {
            return Some(self.client.player_model.as_ref()?.body_yaw);
        }
        None
    }
}

fn mob_category(name: &str) -> MobCategory {
    match name {
        "monster" => MobCategory::Monster,
        "creature" => MobCategory::Creature,
        "ambient" => MobCategory::Ambient,
        "axolotls" => MobCategory::Axolotls,
        "underground_water_creature" => MobCategory::UndergroundWaterCreature,
        "water_creature" => MobCategory::WaterCreature,
        "water_ambient" => MobCategory::WaterAmbient,
        _ => MobCategory::Misc,
    }
}
