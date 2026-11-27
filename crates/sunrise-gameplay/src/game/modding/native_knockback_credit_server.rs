use super::combat_common::PlayerCredit;
use sunrise_gameplay_common::game::native_knockback_credit::CreditEntity;
use sunrise_gameplay_common::game::native_knockback_credit::CreditPlan;
use sunrise_gameplay_common::game::native_knockback_credit::HurtBy;
use sunrise_sim::{
    health::Attack,
    mobs::{Species, Who},
};

impl crate::game::ServerGame<'_> {
    /// The attacking entity's canonical UUID, already assigned on creation or admission.
    pub fn known_attacker_uuid(&self, id: u32) -> Option<[i32; 4]> {
        self.known_resident_entity_uuid_at(self.common.dim.key(), id)
            .or_else(|| self.common.persist.uuids.get(&id).copied())
    }
    /// The credited entity and its main hand: in the level in place, or a mob
    /// a kill credit found in another loaded level.
    pub(in crate::game::server_side) fn knockback_credit_entity(&self, id: i32) -> CreditEntity {
        if id == 0 {
            return CreditEntity::default();
        }
        if self.entity(id as u32).is_some() {
            let [main, _] = self.attack_of(id).held;
            return CreditEntity { id, main };
        }
        match self.mob_in_other_level(id as u32) {
            Some((_, mob)) => CreditEntity {
                id,
                main: mob.extra.main_hand_stack().cloned(),
            },
            None => CreditEntity::default(),
        }
    }
    pub(in crate::game::server_side) fn knockback_player_credit(
        &self,
        credit: PlayerCredit,
    ) -> (PlayerCredit, CreditEntity) {
        let id = self
            .resolve_player_credit(credit)
            .and_then(|connection| self.player_entity_id_by_connection(connection))
            .unwrap_or(0);
        (credit, self.knockback_credit_entity(id))
    }
    /// Capture only this victim's possible credit holders, not a table of every entity.
    /// UUID responsibility is committed by the loan at the accepted-hit memory point.
    pub fn prepare_knockback_credit(
        &self,
        victim: i32,
        attacker: Option<Who>,
        attack: &Attack,
    ) -> CreditPlan {
        let observed = mod_api::__private::listened(mod_api::HookId::LivingKnockback);
        let mob = observed
            .then(|| self.common.mobs.iter().find(|mob| mob.id as i32 == victim))
            .flatten();
        let existing_player = self
            .server
            .last_hurt_by_player
            .get(&(victim as u32))
            .copied();
        let existing_player_live =
            existing_player.is_some_and(|credit| self.resolve_player_credit(credit).is_some());
        let player = mob.and(existing_player);
        let responsibility = match attacker {
            Some(Who::Player) => attack
                .player_uuid
                .map(|uuid| Some(PlayerCredit(sunrise_sim::lan::playerdata::ints_uuid(uuid)))),
            Some(Who::Mob(id)) => self
                .common
                .mobs
                .iter()
                .find(|mob| mob.id == id && mob.species() == Species::Wolf && mob.extra.tame)
                .map(|wolf| {
                    wolf.extra
                        .owner
                        .map(|uuid| PlayerCredit(sunrise_sim::lan::playerdata::ints_uuid(uuid)))
                }),
            None => None,
        };
        let responsibility_live = responsibility
            .flatten()
            .is_some_and(|credit| self.resolve_player_credit(credit).is_some());
        let entity = |id| {
            if observed {
                self.knockback_credit_entity(id)
            } else {
                CreditEntity::default()
            }
        };
        let credited_player = |credit| {
            if observed {
                self.knockback_player_credit(credit)
            } else {
                (credit, CreditEntity::default())
            }
        };
        let previous_mob = mob
            .and_then(|mob| mob.ai.last_hurt_by)
            .and_then(|who| match who {
                Who::Mob(id) => Some(id),
                Who::Player => None,
            })
            .map(|id| {
                let credited = self.credited_entity(id).map_or(0, |e| e.id() as i32);
                (id, entity(credited))
            });
        let hurt_by = self
            .common
            .mobs
            .iter()
            .any(|m| m.id as i32 == victim && m.species() == Species::Villager)
            .then(|| HurtBy {
                // The hit's own player (a guest's cloud's owner), not whoever acts now.
                player: attack.player_uuid,
                attacker: match attacker {
                    Some(Who::Mob(id)) => self.known_attacker_uuid(id).map(|uuid| (id, uuid)),
                    _ => None,
                },
                trade_feedback: attacker == Some(Who::Player)
                    && attack
                        .player_uuid
                        .is_some_and(|uuid| self.villager_trade_will_notify(victim as u32, uuid)),
            });
        let fixed = if observed && mob.is_none() {
            self.entity(victim as u32)
                .and_then(|entity| self.living(entity))
                .and_then(|living| self.living_kill_credit(living))
                .map(|credit| entity(credit.id() as i32))
                .unwrap_or_default()
        } else {
            CreditEntity::default()
        };
        CreditPlan {
            victim,
            player: player.map(credited_player),
            existing_player_live,
            previous_mob: previous_mob.unwrap_or_default(),
            attacking_mob: match attacker {
                Some(Who::Mob(id)) => entity(id as i32),
                _ => CreditEntity::default(),
            },
            responsibility: responsibility.map(|credit| credit.map(credited_player)),
            responsibility_live,
            fixed,
            hurt_by,
        }
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
