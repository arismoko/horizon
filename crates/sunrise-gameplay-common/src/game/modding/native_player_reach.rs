//! A player's reach and flight (#3349): whether an entity is within the server's reach rules,
//! and whether the player may fly (NeoForge's `IPlayerExtension.mayFly`).
//! A client applies the same rules to its local player and the target's displayed box.
use mod_api::{EntityReach, EntityRef, EquipmentSlot, LivingRef, PlayerRef};
use sunrise_core::geometry::{Aabb, Vec3};

pub trait NativePlayerReachGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The server's reach rule `reach` between `player` and `entity`: the squared distance from
    /// the player's eyes to the closest point of the entity's box (`AABB.distanceToSqr`, 0 with
    /// the eyes inside), against the player's `entity_interaction_range` with the rule's buffer,
    /// or for an attack the main hand's attack range (`Player.isWithinAttackRange`). None for a
    /// stale player or entity, one without a box, or one in another level.
    fn player_within_entity_reach(
        &self,
        player: PlayerRef,
        entity: EntityRef,
        reach: EntityReach,
    ) -> Option<bool> {
        let me = player.entity();
        if self.entity_level(me)? != self.entity_level(entity)? {
            return None;
        }
        let v = |[x, y, z]: [f64; 3]| Vec3::new(x, y, z);
        let [lo, hi] = self.entity_bounding_box(entity)?;
        let eye = v(self.entity_eye_position(me)?);
        let distance_squared = Aabb::new(v(lo), v(hi)).distance_squared_to_point(eye);
        let living = LivingRef::new(me);
        let range = self.living_attribute_value(living, "minecraft:entity_interaction_range")?;
        // The server's buffers: 3.0 on its interact and attack checks, 4.0 for a container an
        // entity holds (`ContainerEntity.isChestVehicleStillValid`, the mount inventory).
        let within = |buffer: f64| distance_squared < (range + buffer) * (range + buffer);
        Some(match reach {
            EntityReach::Interact => within(3.0),
            EntityReach::Container => within(4.0),
            EntityReach::Attack => {
                let creative = self.player_is_creative(player)?;
                let held = self
                    .living_equipment_stack(living, EquipmentSlot::MainHand)
                    .map(|stack| stack.base());
                self.stack_attack_range(held, true, creative, range)
                    .contains(distance_squared.sqrt(), 3.0)
            }
        })
    }

    /// `IPlayerExtension.mayFly`: whether the player may start flying, by game mode or ability.
    /// Sunrise has no creative-flight attribute yet, so this is the `mayfly` ability. None for a
    /// stale or unavailable player.
    fn player_may_fly(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_abilities(player)?.may_fly)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativePlayerReachGameExt1<'__sg> for G {}

use crate::game::game_mode_common::GameModeCommonGameExt1 as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use crate::game::native_server::NativeServerGameExt1 as _;
