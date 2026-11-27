//! Spears (`KineticWeapon`): holding the use button charges the spear, and
//! from `delay_ticks` on every living entity along the view that the
//! player's speed allows is stabbed: the damage grows with the speed
//! relative to the target, the knockback and the dismount need the
//! attacker's own speed, and each condition runs out after its time
//! (`Item.use`, `ItemStack.onUseTick`, `KineticWeapon.damageEntities`,
//! `Player.stabAttack`). The hit's feedback (`onKineticHit`) reaches the
//! players around as entity event 2. A left click jabs instead
//! (`PiercingWeapon.attack`), which Lunge turns into a dash.
use sunrise_core::geometry::Aabb;
#[allow(unused_imports)]
pub use sunrise_sim::spear_sim::hits_in_reach;
use {
    sunrise_sim::attack_strength::delay, sunrise_sim::spear_sim::HIT_FEEDBACK_TICKS,
    sunrise_sim::spear_sim::Kinetic,
};

/// The key of this window's own player in `Game::kinetic_feedback`.
pub const OWN: i32 = i32::MIN;

/// What a stab reaches along the view: a mob (its index in `mobs`) or
/// another player (its index in `stab_players`).
#[derive(Clone, Copy)]
pub enum Target {
    Mob(usize),
    Player(usize),
}

impl crate::game::CommonGame<'_> {
    /// The boxes of the mobs a spear can hit: alive, and not the vehicle
    /// ridden (`PiercingWeapon.canHitEntity`), with their index.
    pub fn hittable_mob_boxes(&self) -> Vec<(usize, Aabb)> {
        (self.mobs.iter().enumerate())
            .filter(|(_, m)| m.alive() && !m.removed && self.riding != Some(m.id))
            .map(|(i, m)| (i, m.bounds()))
            .collect()
    }
    /// Record `LivingEntity.onKineticHit` if the feedback cooldown has elapsed.
    pub fn kinetic_hit_feedback(&mut self, entity: i32) -> bool {
        let now = self.world.game_time;
        if self
            .kinetic_feedback
            .get(&entity)
            .is_none_or(|&t| now - t > HIT_FEEDBACK_TICKS)
        {
            self.kinetic_feedback.insert(entity, now);
            return true;
        }
        false
    }
}

pub trait SpearGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `Player.cannotAttackWithItem` for the held item: a spear needs its
    /// `minimum_attack_charge` of 1, counting `tolerance` ticks more than the
    /// `ticker` ticks charged.
    fn cannot_attack_with_item(&self, ticker: i32, tolerance: i32) -> bool {
        self.held().is_some_and(|s| Kinetic::of(&s.item).is_some())
            && (ticker + tolerance) as f32 / delay(self.main_hand_attack_speed()) < 1.0
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> SpearGameExt1<'__sg> for G {}

use crate::game::held_items::HeldItemsGameExt1 as _;
