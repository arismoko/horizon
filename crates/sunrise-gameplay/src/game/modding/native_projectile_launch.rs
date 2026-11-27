//! Owner-created rotational throws, distinct from generic entity summoning.
use super::commands::Target;
use mod_api::{Identifier, LivingRef, ProjectileRef};
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_sim::projectiles::{Projectile, ProjectileKind};

impl crate::game::ServerGame<'_> {
    pub fn spawn_projectile_from_rotation(
        &mut self,
        owner: LivingRef,
        entity_type: &Identifier,
        stack: &ItemStack,
        offset: f32,
        power: f32,
        inaccuracy: f32,
    ) -> Option<ProjectileRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        if !self.common.is_logical_server()
            || ![offset, power, inaccuracy].into_iter().all(f32::is_finite)
        {
            return None;
        }
        let target = self.native_living_target(owner)?;
        if self
            .native_target_guest(target)
            .is_some_and(|guest| guest.dimension != self.common.dim.id())
        {
            return None;
        }
        // A stack whose `projectile_spawned` effects the engine cannot all
        // apply yet (#2670) is refused before anything is made or drawn,
        // rather than spawned without some of them.
        if !self.projectile_spawned_effects_applied(stack) {
            return None;
        }
        let mut carried = stack.clone();
        carried.count = 1;
        let kind = thrown_projectile_kind(&entity_type.to_string(), &carried)?;
        let entity = owner.entity();
        let [x, y, z] = self.entity_eye_position(entity)?;
        let [yaw, pitch] = self.entity_rotation(entity)?;
        let motion = self.native_entity_known_speed(entity)?;
        let on_ground = self.entity_on_ground(entity)?;
        let ((who, uuid, guest), vehicle) = self.native_projectile_owner_fields(Some(entity))?;
        // WindChargeItem's factory uses eye height directly; the other
        // supported factories use ThrowableItemProjectile's widened 0.1F offset.
        let start_y = if matches!(&kind, ProjectileKind::WindCharge) {
            y
        } else {
            y - f64::from(0.1_f32)
        };
        let mut shot = Projectile::new(kind, Vec3::new(x, start_y, z), who);
        (shot.owner_uuid, shot.guest, shot.owner_vehicle) = (uuid, guest, vehicle);
        shot.shoot_from_rotation(pitch, yaw, offset, power, inaccuracy, motion, on_ground);
        shot.item = Some(carried);
        let id = self.spawn_owned_projectile(Some(target), shot, stack)?;
        self.projectile(self.entity(id)?)
    }

    /// `Projectile.spawnProjectile` for a shot whose owner is `owner`. A player
    /// owner may be an inactive guest; resolve that body before inserting the shot
    /// so join listeners see its real owner in the right level.
    pub(in crate::game::server_side) fn spawn_owned_projectile(
        &mut self,
        owner: Option<Target>,
        shot: Projectile,
        stack: &ItemStack,
    ) -> Option<u32> {
        match owner {
            Some(t @ (Target::Player | Target::Guest(_))) => {
                self.for_player(t, |game| game.spawn_projectile(shot, stack))?
            }
            _ => self.spawn_projectile(shot, stack),
        }
    }
}

/// The thrown item projectile `entity_type` names, carrying `carried`:
/// vanilla's throwables or a content thrown type. None for any other type.
pub(in crate::game::server_side) fn thrown_projectile_kind(
    entity_type: &str,
    carried: &ItemStack,
) -> Option<ProjectileKind> {
    Some(match entity_type {
        "minecraft:snowball" => ProjectileKind::Snowball,
        "minecraft:egg" => ProjectileKind::Egg,
        "minecraft:ender_pearl" => ProjectileKind::EnderPearl,
        "minecraft:experience_bottle" => ProjectileKind::ExperienceBottle,
        "minecraft:wind_charge" => ProjectileKind::WindCharge,
        "minecraft:splash_potion" => ProjectileKind::Potion(Box::new(carried.clone()), false),
        "minecraft:lingering_potion" => ProjectileKind::Potion(Box::new(carried.clone()), true),
        id => ProjectileKind::Thrown(sunrise_sim::projectiles::thrown_type(id)?),
    })
}

use sunrise_gameplay_common::game::native_entity_motion_values::NativeEntityMotionValuesGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
