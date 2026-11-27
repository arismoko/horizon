//! Accepted melee hits on special targets and their shared attacker tail.
use super::{dragon::Hurt, *};
use crate::game::server_side::minecarts_server::CartHit;
use sunrise_sim::mobs::DamageKind;

impl crate::game::ServerGame<'_> {
    /// Player.attack's cost after an accepted special-entity hurt. Hanging
    /// decorations consume their hit in skipAttackInteraction, before this tail.
    pub(in crate::game::server_side) fn special_attack_exhaustion(&mut self) {
        if let Some(player) = self.common.player.as_mut() {
            player.cause_food_exhaustion(0.1);
        }
    }
    /// The attacker's side of Player.causeExtraKnockback, including when the
    /// accepted target was removed by its hurt implementation.
    pub(in crate::game::server_side) fn special_attack_slowdown(&mut self, knockback: f32) {
        if knockback > 0.0 {
            self.common.attacker_slowdown();
        }
    }

    pub(in crate::game::server_side) fn attack_dragon(&mut self) -> bool {
        let Some((id, d)) = self.dragon_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        // A dragon part is no LivingEntity: `canCriticalAttack` is false.
        let (amount, scale, _, _) = self.melee_damage(
            "minecraft:ender_dragon",
            false,
            self.common.dragon.as_ref().map(|dragon| dragon.id),
        );
        self.swing_arm();
        // The part's box.
        let area = self
            .common
            .special_targets()
            .into_iter()
            .find(|(part, _)| *part == id)
            .map(|(_, b)| b);
        // The sweep and the sounds read the charge, which the hit then spends.
        let sweep = self.sweep_before_hit("minecraft:ender_dragon", false);
        let hurt = self.hit_special(id, amount, true, DamageKind::PlayerAttack, None);
        if let (Some(area), Hurt::Landed { .. }) = (area, hurt) {
            let (knockback, _) = self.extra_knockback(
                "minecraft:ender_dragon",
                self.dragon.as_ref().map_or(0, |dragon| dragon.id),
                scale,
                None,
            );
            self.attack_effects(area, sweep, None, false, super::mace::Spared::Nothing, None);
            self.remember_player_target(None);
            self.post_attack_dragon_burst();
            self.special_attack_slowdown(knockback);
            self.special_attack_exhaustion();
        }
        self.server.attack_ticks = 0;
        true
    }
    /// `VehicleEntity.hurtServer` from the player's attack.
    pub(in crate::game::server_side) fn attack_cart(&mut self) -> bool {
        let Some((i, d)) = self.cart_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        let burning = self
            .common
            .held()
            .is_some_and(|s| s.enchantment_level("minecraft:fire_aspect") > 0);
        let area = self.common.carts[i].bounds();
        // Player.attack's full damage against the cart's entity type.
        let id = self.common.carts[i].id as i32;
        let kind = self.common.carts[i].kind.id();
        let (damage, scale, _, magic) = self.melee_damage(kind, false, Some(id as u32));
        let sweep = self.sweep_before_hit(kind, false);
        let attacker = self.own_entity_id();
        let source = sunrise_sim::health::HurtSource {
            damage_type: "minecraft:player_attack",
            causing: attacker,
            direct: attacker,
            position: None,
            direct_type: None,
        };
        let vanilla =
            self.saved_refuses_damage(id as u32, "minecraft:player_attack", self.common.creative);
        if !crate::game::entity_server::damage_level_of!(
            self,
            self.server.combat.0.attack(),
            no_push,
            None
        )
        .invulnerability_check(
            sunrise_sim::health::Hurting::Entity(id, kind, source),
            vanilla,
        ) && self.hurt_cart_result(i, damage, self.common.creative, burning, CartHit::player())
            && damage > 0.0
        {
            let (knockback, _) =
                self.extra_knockback(kind, id as u32, scale, Some("minecraft:player_attack"));
            self.special_attack_slowdown(knockback);
            self.remember_player_target(None);
            self.special_attack_exhaustion();
        }
        self.vehicle_attack_sounds(area, sweep, magic.then_some(id));
        // Player.onAttack: the hit starts the attack strength again.
        self.server.attack_ticks = 0;
        self.swing_arm();
        true
    }
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
