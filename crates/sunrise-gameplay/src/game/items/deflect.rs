//! A player's blow at a ghast's fireball or a wind charge
//! (`#minecraft:redirectable_projectile`): `Player.attack` turns it with
//! `ProjectileDeflection.AIM_DEFLECT` instead of hurting anything. It flies on
//! along the player's view and is the player's now (`Projectile.deflect`
//! sets the owner), so a ghast it meets dies of it (`Ghast.isReflectedFireball`).
use crate::game::ServerGame;
use sunrise_core::geometry::Vec3;
use sunrise_sim::{
    mobs::Who,
    projectiles::{Projectile, ProjectileKind, Turn},
};

/// `Projectile.onRedirectProjectile`: another projectile hit `p`, which turns
/// with `deflect(AIM_DEFLECT, hitter's owner, hitter's owner, true, 1.0)`.
/// It flies along the owner's look (`Entity.getLookAngle`; unchanged when the
/// owner is gone), is that owner's now, and its acceleration power is back
/// to 0.1 (`AbstractHurtingProjectile.onDeflection` by an attack). A wind
/// charge in its first five ticks refuses (`WindCharge.deflect`): false, and
/// nothing changes.
pub(in crate::game::server_side) fn aim_deflect(
    p: &mut Projectile,
    look: Option<Vec3>,
    owner: Option<Who>,
    owner_uuid: Option<[i32; 4]>,
    guest: Option<u32>,
) -> bool {
    p.deflect(
        Turn::Direction(look),
        [1.0; 3],
        (owner, owner_uuid, guest),
        true,
    )
}

impl ServerGame<'_> {
    /// The pickable projectile under the crosshair within entity reach.
    pub(in crate::game::server_side) fn redirectable_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = super::pick_ray::pick_ray_segment(eye, dir, reach);
        let mut pick = super::pick_ray::PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, p) in self.common.creatures.projectiles.iter().enumerate() {
            if !self.projectile_is_pickable(p) {
                continue;
            }
            let radius = self.projectile_pick_radius(p);
            let bb = if radius == 0.0 {
                p.bounds()
            } else {
                p.bounds().inflate(Vec3::new(radius, radius, radius))
            };
            if pick.consider_box(self, p.id, from, to, bb) {
                best = Some((i, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    /// `Player.attack` on a redirectable projectile nearer than any mob or
    /// block in view: `AIM_DEFLECT` (its movement the unit view vector), the
    /// player its owner, its acceleration power back to 0.1
    /// (`onDeflection` by an attack), and the no-damage sound. True when a
    /// projectile was turned.
    pub(in crate::game::server_side) fn attack_redirectable(&mut self) -> bool {
        let Some((i, t)) = self.redirectable_hit() else {
            return false;
        };
        if self.mob_hit().is_some_and(|(_, d, _)| d < t) {
            return false;
        }
        if !self.before_block(t) {
            return false;
        }
        if matches!(
            self.creatures.projectiles[i].kind,
            ProjectileKind::ShulkerBullet { .. }
        ) {
            self.hit_shulker_bullet(i);
        } else {
            self.deflect(i);
        }
        true
    }

    /// `ShulkerBullet.hurtServer` from a player's blow: the hurt sound, fifteen crit
    /// particles (`sendParticles`, spread 0.2) and `destroy`. The blow is
    /// `Player.attack` on a target that is not a mob: its sounds, then the swing.
    pub(in crate::game) fn hit_shulker_bullet(&mut self, i: usize) {
        let ((_, _, _, magic), admitted) = self.melee_damage_with_admission(
            "minecraft:shulker_bullet",
            false,
            Some(self.common.creatures.projectiles[i].id),
        );
        let sweep = self.sweep_before_hit("minecraft:shulker_bullet", false);
        let p = self.destroy_shulker_bullet(i);
        self.vehicle_attack_sounds(p.aabb(), sweep, magic.then_some(p.id as i32));
        if admitted {
            self.remember_player_target(None);
        }
        self.reset_attack_strength();
        self.swing_arm();
    }

    /// `ShulkerBullet.hurtServer`, whatever hurt it: the hurt sound, fifteen
    /// crit particles (`sendParticles`, spread 0.2), then `destroy`
    /// (`discard` and `gameEvent(ENTITY_DAMAGE)` from the bullet).
    pub fn destroy_shulker_bullet(&mut self, i: usize) -> Projectile {
        let (id, at, owner, owner_uuid, silent) = {
            let p = &self.common.creatures.projectiles[i];
            (
                p.id,
                [p.position.x, p.position.y, p.position.z],
                p.owner,
                p.owner_uuid,
                self.entity_silent(p.id),
            )
        };
        let ctx = sunrise_level::sculk::Context {
            owner,
            owner_uuid,
            ..self.entity_event_context(id, None)
        };
        self.shulker_bullet_hurt(at, silent);
        let p = self.remove_projectile_at(i);
        self.post_game_event("minecraft:entity_damage", at, ctx);
        p
    }

    /// `Player.canInteractWithEntity(box, 3.0)` as the server checks an
    /// attack packet: the projectile's box within the reach plus 3 of the eyes.
    pub(in crate::game::server_side) fn within_interaction_range(&self, i: usize) -> bool {
        let (Some(eye), Some(p)) = (
            self.common.eye_position(),
            self.common.creatures.projectiles.get(i),
        ) else {
            return false;
        };
        let b = p.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(lo[a], hi[a])).powi(2))
            .sum();
        let range = self.common.entity_reach + 3.0;
        d2 < range * range
    }

    /// `Entity.getLookAngle` of a projectile's owner: the host, the guest who
    /// threw it, or a mob; none when the owner is not here.
    pub(in crate::game) fn owner_look(
        &self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> Option<Vec3> {
        let (pitch, yaw) = match (owner?, guest) {
            (Who::Player, Some(conn)) => {
                let lan = self.server.lan.as_ref()?;
                let g = lan.hub.guests.iter().find(|g| g.conn == conn)?;
                (g.pitch, g.yaw)
            }
            (Who::Player, None) => {
                self.common.player.as_ref()?;
                (self.common.pitch, self.common.yaw)
            }
            (Who::Mob(id), _) => {
                let m = self.common.mobs.iter().find(|m| m.id == id)?;
                (m.head_pitch, m.yaw)
            }
        };
        Some(sunrise_world::view_direction::view_vector(pitch, yaw))
    }

    /// The deflection itself: the view as the new movement, the player as
    /// owner. False, and nothing changes, when the projectile refuses.
    pub(in crate::game::server_side) fn turn_projectile(&mut self, i: usize) -> bool {
        let (pitch, yaw) = (self.common.pitch, self.common.yaw);
        let (guest, owner_uuid) = (self.server.lan_acting, Some(self.acting_player_uuid()));
        self.common
            .creatures
            .projectiles
            .update(i, |p| attack_deflect(p, pitch, yaw, owner_uuid, guest))
    }
}

/// `Player.attack`'s `deflect(AIM_DEFLECT, player, player, true)`: [`aim_deflect`] along the
/// player's look at `pitch` and `yaw` (`Entity.getLookAngle`, vanilla's sine table), with `p`
/// turned to it. False, and nothing changes, when the projectile refuses.
pub(in crate::game::server_side) fn attack_deflect(
    p: &mut Projectile,
    pitch: f32,
    yaw: f32,
    owner_uuid: Option<[i32; 4]>,
    guest: Option<u32>,
) -> bool {
    let look = sunrise_world::view_direction::view_vector(pitch, yaw);
    if !aim_deflect(p, Some(look), Some(Who::Player), owner_uuid, guest) {
        return false;
    }
    // As Projectile.updateRotation turns it: Mth.atan2 times the folded 180F / (float)PI.
    p.yaw = sunrise_core::geometry::trig::atan2_degrees(look.x, look.z);
    p.pitch = sunrise_core::geometry::trig::atan2_degrees(look.y, look.horizontal_distance());
    // Not into the one who turned it on its way out.
    p.left_owner = false;
    true
}

impl ServerGame<'_> {
    /// `Player.attack` on a redirectable projectile: `onAttack` resets the
    /// attack strength, then `Projectile.deflect(AIM_DEFLECT, player, player,
    /// true)`. When the projectile refuses (a wind charge in its first five
    /// ticks) the blow goes on as an ordinary attack, which a projectile does
    /// not take: `hurtOrSimulate` fails and `PLAYER_ATTACK_NODAMAGE` plays,
    /// the same sound. Either way the swing follows. True when it turned.
    pub fn deflect(&mut self, i: usize) -> bool {
        self.reset_attack_strength();
        let turned = self.turn_projectile(i);
        self.swing_arm();
        let from = self
            .common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if let Some(at) = from {
            self.server_play_sound("minecraft:entity.player.attack.nodamage", at, 1.0, 1.0);
        }
        turned
    }
    /// `ShulkerBullet.hurtServer`'s sound (unless silent) and fifteen crit particles
    /// (`sendParticles`, spread 0.2) at the bullet, before it is discarded.
    pub(in crate::game) fn shulker_bullet_hurt(&mut self, at: [f64; 3], silent: bool) {
        if !silent {
            self.server_play_sound("minecraft:entity.shulker_bullet.hurt", at, 1.0, 1.0);
        }
        if let Some(ty) = sunrise_level::particle_options::ParticleType::from_name("crit") {
            self.server_level_particles(
                &sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                at,
                [0.2, 0.2, 0.2],
                0.0,
                15,
            );
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_core::geometry::Aabb;
    use sunrise_gameplay_common::game::deflect_common::{PICK_RADIUS, attackable, redirectable};

    /// `Projectile.getPickRadius`: `isPickable() ? 1.0F : 0.0F` (`ShulkerBullet` inherits this).
    fn pick_radius(p: &Projectile) -> f64 {
        if attackable(p) { PICK_RADIUS } else { 0.0 }
    }

    /// Its box for the crosshair: the entity's box inflated by `Projectile.getPickRadius` (1.0 when pickable).
    fn pick_box(p: &Projectile) -> Aabb {
        let radius = pick_radius(p);
        p.bounds().inflate(Vec3::new(radius, radius, radius))
    }

    #[test]
    fn only_ghast_fireballs_and_wind_charges_are_redirectable() {
        let at = Vec3::new(0.0, 64.0, 0.0);
        let large = Projectile::fireball(
            ProjectileKind::LargeFireball { power: 1 },
            at,
            None,
            [1.0, 0.0, 0.0],
        );
        let small = Projectile::fireball(ProjectileKind::SmallFireball, at, None, [1.0, 0.0, 0.0]);
        assert!(redirectable(&large));
        assert!(!redirectable(&small));
        // A shulker bullet is pickable (`ShulkerBullet.isPickable`) and inherits `Projectile.getPickRadius` (1.0).
        let bullet = Projectile::new(
            ProjectileKind::ShulkerBullet {
                target: Some(Who::Player),
                dir: None,
                steps: 0,
                delta: [0.0; 3],
                seed: 0,
            },
            at,
            None,
        );
        assert!(attackable(&bullet) && !redirectable(&bullet));
        assert!(!attackable(&small));
        let b = pick_box(&bullet);
        assert_eq!(b.min_position().y, 63.0);
        assert_eq!(b.max_position().y, 65.3125);
        // The pick box: a block round the fireball's 1 x 1 box.
        let b = pick_box(&large);
        assert_eq!(b.min_position().x, -1.5);
        assert_eq!(b.max_position().y, 66.0);
    }

    /// A wind charge past its first five ticks (`noDeflectTicks` run out).
    fn wind_charge() -> Projectile {
        let mut p = Projectile::new(
            ProjectileKind::WindCharge,
            Vec3::new(0.0, 70.0, 0.0),
            Some(Who::Mob(7)),
        );
        p.velocity = Vec3::new(0.0, 1.5, 0.0);
        // Past `WindCharge.noDeflectTicks`.
        p.no_deflect_ticks = 0;
        p
    }

    #[test]
    fn a_new_wind_charge_refuses_a_redirect_for_five_ticks() {
        let mut p = wind_charge();
        p.no_deflect_ticks = 5;
        let look = Vec3::new(0.0, -1.0, 0.0);
        assert!(!aim_deflect(
            &mut p,
            Some(look),
            Some(Who::Player),
            None,
            None
        ));
        assert_eq!(p.velocity, Vec3::new(0.0, 1.5, 0.0));
        assert_eq!(p.owner, Some(Who::Mob(7)));
        assert!(!p.needs_sync);
        p.no_deflect_ticks = 0;
        assert!(aim_deflect(
            &mut p,
            Some(look),
            Some(Who::Player),
            None,
            None
        ));
        assert_eq!(p.velocity, look);
    }

    /// A player's blow sends it along `Entity.calculateViewVector(0, 0.001)`, on HotSpot 25.0.4.1
    /// (0.0, -0.0, 1.0) as double bits: `Mth.sin`'s table leaves no x at that yaw.
    #[test]
    fn a_blow_turns_it_along_vanillas_look() {
        let mut p = wind_charge();
        assert!(attack_deflect(&mut p, 0.0, 0.001, None, None));
        assert_eq!(
            [p.velocity.x, p.velocity.y, p.velocity.z].map(f64::to_bits),
            [0, 0x8000_0000_0000_0000, 0x3ff0_0000_0000_0000]
        );
    }

    #[test]
    fn a_redirected_wind_charge_flies_along_the_new_owners_look() {
        let mut p = wind_charge();
        let look = Vec3::new(0.0, -1.0, 0.0);
        aim_deflect(&mut p, Some(look), Some(Who::Player), Some([4; 4]), Some(3));
        assert_eq!(p.velocity, look);
        assert_eq!((p.owner, p.owner_uuid), (Some(Who::Player), Some([4; 4])));
        assert_eq!(p.guest, Some(3));
        assert_eq!(p.acceleration, 0.1);
        assert!(p.needs_sync);
    }

    #[test]
    fn with_no_owner_here_it_keeps_its_movement_but_changes_hands() {
        let mut p = wind_charge();
        p.owner_uuid = Some([7; 4]);
        aim_deflect(&mut p, None, None, None, None);
        assert_eq!(p.velocity, Vec3::new(0.0, 1.5, 0.0));
        assert_eq!((p.owner, p.owner_uuid), (None, None));
        assert_eq!(p.acceleration, 0.1);
        assert!(!p.needs_sync);
    }
}
