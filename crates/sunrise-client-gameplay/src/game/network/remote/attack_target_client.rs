use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Player.attack`'s damage and `getAttackStrengthScale(0.5)` on this
    /// client.
    pub(in crate::game) fn remote_attack_damage(&mut self) -> (f32, f32) {
        // `getAttackStrengthScale(0.5)`: here the ticker is `hand_strength`'s
        // (`attack_ticks` only counts in the local world), and the attack
        // speed is the player's synced attribute, which a server may change.
        let speed = self
            .own_attribute("minecraft:attack_speed")
            .unwrap_or_else(|| self.main_hand_attack_speed());
        let strength = self.client.hand_strength.scale(0.5, speed);
        // Player.attack's damage: a riptide spin's, or this client's own
        // ATTACK_DAMAGE, which the held item and effects don't change here.
        let spin = self
            .common
            .player
            .as_ref()
            .map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 {
            spin
        } else {
            self.own_attribute("minecraft:attack_damage")
                .map_or(1.0, |v| v as f32)
        };
        (damage, strength)
    }

    /// `Minecraft.startAttack`'s range check and `MultiPlayerGameMode.attack`'s send and prediction.
    pub fn remote_attack_entity(&mut self, id: i32, distance: f64) -> bool {
        if !self.held_range_allows(distance) {
            return false;
        }
        self.send(S::Attack(play::Attack { id }));
        if let Some(index) = self.common.mobs.iter().position(|mob| mob.id as i32 == id) {
            let kind = format!(
                "minecraft:{}",
                self.mobs[index].kind.id.trim_start_matches("minecraft:")
            );
            self.remote_sweep_event(&kind, true);
            self.pvp_attacked(id, distance);
            self.remote_player_attack(index);
        } else if let Some(projectile) = self
            .common
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id as i32 == id)
        {
            if !sunrise_gameplay_common::game::deflect_common::redirectable(projectile) {
                let bullet = matches!(
                    projectile.kind,
                    sunrise_sim::projectiles::ProjectileKind::ShulkerBullet { .. }
                );
                self.remote_sweep_event("minecraft:shulker_bullet", false);
                if bullet {
                    self.remote_object_attack();
                }
            }
        } else if let Some((kind, living)) = self.remote_attacked_object(id) {
            self.remote_sweep_event(&kind, living);
            // A boat, a minecart or an End crystal; an armor stand is
            // living, and its hurtClient is false.
            if !living {
                self.remote_object_attack();
            }
        } else if let Some(kind) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.general.rows.get(&id))
            .map(|row| row.kind())
        {
            // A row reaches Player.attack but hurtClient is false.
            self.remote_sweep_event(kind, false);
        }
        self.reset_attack_strength();
        if let Some(remote) = self.client.remote.as_mut()
            && remote.abilities.instabuild
        {
            remote.destroy_delay = 5;
        }
        true
    }
    /// The attacker's side of `causeExtraKnockback` after a hit whose
    /// `hurtClient` is true on this client (another player, a boat, a
    /// minecart, an End crystal, a shulker bullet): with any knockback, the
    /// 0.6 horizontal speed cut and the sprint stop, which that tick's
    /// `sendChanges` sends. Without it the next move outruns the server's
    /// player.
    pub(in crate::game) fn remote_attacker_knockback(&mut self, damage: f32, strength: f32) {
        let knockback = self
            .own_attribute("minecraft:attack_knockback")
            .map_or(0.0, |v| v as f32);
        let sprinting = self.common.player.as_ref().is_some_and(|p| p.sprinting);
        if sunrise_sim::attack_strength::attacker_slows_after_hit(
            damage, knockback, strength, sprinting,
        ) {
            self.common.attacker_slowdown();
        }
    }
    /// A hit on a boat, minecart, End crystal or shulker bullet: their
    /// `hurtClient` is true (`VehicleEntity`, `EndCrystal` for any source but
    /// the dragon, `ShulkerBullet`), so `Player.attack` reaches
    /// `causeExtraKnockback` here as for a player.
    pub(in crate::game) fn remote_object_attack(&mut self) {
        let (damage, strength) = self.remote_attack_damage();
        self.remote_attacker_knockback(damage, strength);
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
