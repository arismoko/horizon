//! Narrow, per-victim context prepared before the hit borrows its victim.
use super::combat_common::PlayerCredit;
use std::collections::HashMap;
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    health::{Attack, Hurting, KnockbackCredit},
    mobs::{Damage, Mob, Who},
};

#[derive(Default)]
pub struct CreditEntity {
    pub id: i32,
    pub main: Option<ItemStack>,
}

pub struct CreditPlan {
    pub victim: i32,
    pub player: Option<(PlayerCredit, CreditEntity)>,
    pub existing_player_live: bool,
    /// The mob the victim's hurt memory names, by that recorded id, and the
    /// entity the credit resolves to now (`LivingRef::kill_credit`'s).
    pub previous_mob: (u32, CreditEntity),
    pub attacking_mob: CreditEntity,
    pub responsibility: Option<Option<(PlayerCredit, CreditEntity)>>,
    pub responsibility_live: bool,
    pub fixed: CreditEntity,
    /// Canonical UUIDs of the causing entity for a villager's hurt gossip.
    pub hurt_by: Option<HurtBy>,
}

pub struct HurtBy {
    pub player: Option<[i32; 4]>,
    pub attacker: Option<(u32, [i32; 4])>,
    pub trade_feedback: bool,
}

pub struct CreditLoan<'a> {
    pub(in crate::game) plan: CreditPlan,
    pub(in crate::game) players: &'a mut HashMap<u32, PlayerCredit>,
}

impl CreditPlan {
    pub fn lend(self, players: &mut HashMap<u32, PlayerCredit>) -> CreditLoan<'_> {
        CreditLoan {
            plan: self,
            players,
        }
    }
}

impl KnockbackCredit for CreditLoan<'_> {
    fn accepted_mob_hit(&mut self, mob: &mut Mob) {
        assert_eq!(mob.id as i32, self.plan.victim);
        let interval = mob.ambient_interval();
        if let Some(hurt) = &mut self.plan.hurt_by
            && std::mem::take(&mut hurt.trade_feedback)
            && let Some(pending) = mob
                .extra
                .villager
                .as_mut()
                .and_then(|ext| ext.pending_hurt_player.as_mut())
        {
            // notifyTradeUpdated draws before the victim's own hurt sound or effects.
            pending.trade_pitch = sunrise_sim::merchant::trade_update_pitch(
                &mut mob.ambient_timer,
                interval,
                &mut mob.random,
            );
        }
        if let Some(responsibility) = self.plan.responsibility.take() {
            mob.ai.last_hurt_by_player = if responsibility.is_some() { 100 } else { 0 };
            match &responsibility {
                Some((credit, _)) => {
                    self.players.insert(mob.id, *credit);
                }
                None => {
                    self.players.remove(&mob.id);
                }
            }
            self.plan.player = responsibility;
        }
    }

    fn hurt_by_uuid(&self, source: &Damage) -> Option<[i32; 4]> {
        let hurt_by = self.plan.hurt_by.as_ref()?;
        match source.attacker? {
            Who::Player => hurt_by.player,
            Who::Mob(id) => hurt_by
                .attacker
                .filter(|&(attacker, _)| attacker == id)
                .map(|(_, uuid)| uuid),
        }
    }

    fn resolve_player_responsible(&mut self, mob: &mut Mob) -> bool {
        assert_eq!(mob.id as i32, self.plan.victim);
        let updated = self.plan.responsibility.is_some();
        self.accepted_mob_hit(mob);
        if updated {
            self.plan.responsibility_live
        } else {
            self.players.contains_key(&mob.id) && self.plan.existing_player_live
        }
    }

    fn snapshot<'a>(&'a self, pushed: &Hurting<'_>, _: &'a Attack) -> (i32, Option<&'a ItemStack>) {
        let selected = match pushed {
            Hurting::Mob(mob, _) => {
                assert_eq!(mob.id as i32, self.plan.victim);
                if self.players.contains_key(&mob.id) {
                    // An unresolved remembered player still takes precedence over a mob.
                    self.plan.player.as_ref().map(|(_, entity)| entity)
                } else {
                    match mob.recent_hurt_by() {
                        Some(Who::Mob(id)) if id as i32 == self.plan.attacking_mob.id => {
                            Some(&self.plan.attacking_mob)
                        }
                        Some(Who::Mob(id)) if id == self.plan.previous_mob.0 => {
                            Some(&self.plan.previous_mob.1)
                        }
                        _ => None,
                    }
                }
            }
            _ => Some(&self.plan.fixed),
        };
        selected.map_or((0, None), |entity| (entity.id, entity.main.as_ref()))
    }
}
