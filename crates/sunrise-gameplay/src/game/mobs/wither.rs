//! The wither on the game side: building it (`WitherSkullBlock.checkSpawn`),
//! its boss bar, and its skulls hitting and exploding.
use crate::game::ServerGame;
use sunrise_core::{geometry::Direction, position::BlockPos};
use sunrise_gameplay_common::game::dragon_common::SPECIAL;
use sunrise_items::items::ItemStack;
use sunrise_sim::block_pattern::{BlockPattern, PatternMatch, WITHER_FULL};
use sunrise_sim::{
    mobs::{Damage, DamageKind, Species, Who},
    projectiles::Projectile,
};
use sunrise_world::registry::BlockStateId;

impl ServerGame<'_> {
    /// `LivingEntity.createWitherRose`: a mob a wither killed leaves a wither
    /// rose where it stood (with mobGriefing, on air that can hold it), else
    /// drops one.
    pub(in crate::game) fn create_wither_rose(&mut self, m: &sunrise_sim::mobs::Mob) {
        // getKillCredit: the player who last hurt it, else the last mob (kept
        // for 100 ticks).
        let wither = match m.ai.last_hurt_by {
            Some(Who::Mob(id))
                if m.ai.last_hurt_by_player <= 0
                    && m.age as i32 - m.ai.last_hurt_by_time <= 100
                    && self.is_wither(id) =>
            {
                Some(id)
            }
            _ => None,
        };
        let position = [m.position.x, m.position.y, m.position.z];
        self.create_wither_rose_at(position, wither);
    }
    /// `LivingEntity.createWitherRose` for a dying player (`ServerPlayer.die`),
    /// whose kill credit is `killer`.
    pub(in crate::game::server_side) fn create_wither_rose_by(
        &mut self,
        position: [f64; 3],
        killer: Option<u32>,
    ) {
        let wither = killer.filter(|&id| self.is_wither(id));
        self.create_wither_rose_at(position, wither);
    }
    /// `getKillCredit` naming a wither that is still in the level.
    fn is_wither(&self, id: u32) -> bool {
        self.common
            .mobs
            .iter()
            .any(|o| o.id == id && o.alive() && o.species() == Species::Wither)
    }
    fn create_wither_rose_at(&mut self, position: [f64; 3], wither: Option<u32>) {
        let Some(wither) = wither else {
            return;
        };
        let pos = [
            position[0].floor() as i32,
            position[1].floor() as i32,
            position[2].floor() as i32,
        ];
        let registry = self.common.pack.assets.registry;
        let rose = registry.default_state("minecraft:wither_rose");
        // NeoForge: `canEntityGrief` for the wither.
        if self.can_entity_grief(Some(wither))
            && let Some(rose) = rose
            && self
                .common
                .res
                .facts()
                .is_air(self.common.world.block(pos[0], pos[1], pos[2]))
            && self.with_level(|b, level, ticks| {
                b.can_survive(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]), rose)
            })
        {
            let _ = self.level_set(pos, rose);
            return;
        }
        self.spawn_item_own(ItemStack::new("minecraft:wither_rose", 1), position, 0);
    }
    fn is_wither_skull_state(&self, state: BlockStateId) -> bool {
        matches!(
            &*self.res.facts().get(state).name,
            "minecraft:wither_skeleton_skull" | "minecraft:wither_skeleton_wall_skull"
        )
    }

    /// `pattern.find(level, pos)` for the wither patterns: '^' a wither
    /// skeleton skull, '#' a summon base block, '~' air. `at_origin` can
    /// substitute a proposed state without putting it in the world.
    pub(in crate::game::server_side) fn find_wither_pattern(
        &self,
        pattern: BlockPattern,
        pos: [i32; 3],
        at_origin: BlockStateId,
    ) -> Option<PatternMatch> {
        pattern.find(pos, |key, p| {
            let state = if p == pos {
                at_origin
            } else {
                self.common.world.block(p[0], p[1], p[2])
            };
            match key {
                '^' => self.is_wither_skull_state(state),
                '#' => self
                    .common
                    .res
                    .items()
                    .block_tags
                    .get("minecraft:wither_summon_base_blocks")
                    .is_some_and(|tag| tag.contains(&*self.common.res.facts().get(state).name)),
                _ => self.common.res.facts().is_air(state),
            }
        })
    }

    /// The same spawn eligibility for an actual or proposed skull placement.
    pub(in crate::game) fn wither_spawn_pattern(
        &self,
        placed: [i32; 3],
        state: BlockStateId,
    ) -> Option<(PatternMatch, Vec<[i32; 3]>)> {
        if self.common.ui.difficulty == 0 || !self.is_wither_skull_state(state) {
            return None;
        }
        let found = self.find_wither_pattern(WITHER_FULL, placed, state)?;
        let cells = found.front_cells();
        if cells
            .iter()
            .any(|pos| self.common.world.carrier_fences.locks_cell(pos))
        {
            return None;
        }
        Some((found, cells))
    }

    /// `WitherSkullBlock.checkSpawn`: the T of three skulls on four soul sand
    /// or soul soil, built in any orientation, becomes a wither.
    pub(in crate::game) fn check_wither_spawn(
        &mut self,
        placed: [i32; 3],
    ) -> sunrise_game_data::Result<()> {
        // A chosen placement captures the pattern clears in its savepoint. Other
        // transactional routes still do not stage the wither callback.
        let staged = self.captures_placement();
        if !self.common.transactions.is_empty() && !staged {
            return Ok(());
        }
        let state = self.common.world.block(placed[0], placed[1], placed[2]);
        let Some((found, cells)) = self.wither_spawn_pattern(placed, state) else {
            return Ok(());
        };
        if staged {
            self.clear_pattern_blocks(&cells)?;
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::WitherPattern(found, cells),
            );
            return Ok(());
        }
        // Clear and update the formation before the item notice, without publishing
        // unrelated effects that were already queued (notably the item's place sound
        // and block_place event). Restore those even if pattern processing fails.
        let pending = std::mem::take(&mut self.common.level_events);
        let result = self
            .clear_pattern_blocks(&cells)
            .and_then(|()| self.finish_wither_pattern(found, &cells));
        self.common.level_events.splice(..0, pending);
        result
    }

    pub(in crate::game) fn finish_wither_pattern(
        &mut self,
        found: PatternMatch,
        cells: &[[i32; 3]],
    ) -> sunrise_game_data::Result<()> {
        // The cells were cleared before spawning. The wither appears on the middle of
        // the bottom row facing along the pattern's forwards axis, then neighbours update.
        let base = found.block(1, 2, 0);
        let at = [
            f64::from(base[0]) + 0.5,
            f64::from(base[1]) + 0.55,
            f64::from(base[2]) + 0.5,
        ];
        let yaw = if matches!(found.forwards, Direction::West | Direction::East) {
            0.0
        } else {
            90.0
        };
        let id = self.create_triggered("minecraft:wither", at, yaw);
        if let Some(m) = id.and_then(|id| self.common.mobs.iter_mut().find(|m| m.id == id)) {
            m.persistent = true;
            sunrise_sim::mobs::wither::make_invulnerable(m);
        }
        // SUMMONED_ENTITY for each player within 50 blocks of the wither's box.
        self.trigger_summoned(id, 50.0);
        self.update_pattern_blocks(cells)
    }
    /// `WitherSkull.onHitEntity` / `onHit`: 8 `wither_skull` damage from a
    /// skull with an owner (healing a wither owner 5 when it kills), else 5
    /// `magic`, withering on a living target that took it (10 seconds on
    /// Normal, 40 on Hard, none on Easy), then a power 1 blast.
    pub fn wither_skull_hit(
        &mut self,
        p: &Projectile,
        hit: sunrise_sim::projectiles::Hit,
        at: [f64; 3],
    ) -> sunrise_game_data::Result<()> {
        let who = match hit {
            sunrise_sim::projectiles::Hit::Entity { who, .. } => Some(who),
            _ => None,
        };
        // Projectile.getOwner: none once the owner is gone.
        let live_owner = self.resolved_projectile_owner(p.owner, p.guest);
        let owner = match live_owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        // A live player owner is the lent attack's attacker; no owner at all
        // names no causing entity (`magic()`), even though the skull's attack
        // still carries the dead shooter.
        let cause = match live_owner {
            Some(Who::Mob(id)) => sunrise_gameplay_common::game::creatures::KnownCause::Entity(id),
            Some(Who::Player) => sunrise_gameplay_common::game::creatures::KnownCause::Attack,
            None => sunrise_gameplay_common::game::creatures::KnownCause::None,
        };
        let from = [p.previous.x, p.previous.y, p.previous.z];
        let skull = skull_source(live_owner, from);
        let (amount, kind) = (skull.amount, skull.kind);
        let wither_ticks = match self.difficulty_id() {
            2 => 200,
            3 => 800,
            _ => 0,
        };
        match who {
            Some(Who::Player) => {
                // WitherSkull.onHitEntity's source: the wither launched
                // it, and `magic` names no entity and no position.
                self.server.combat.0.projectile_owner = owner;
                if skull.from.is_none() {
                    self.server.combat.0.projectile_direct = None;
                }
                let damage_type =
                    sunrise_gameplay_common::game::creatures::damage_type(kind, false);
                let hit = self.hurt_player_from(amount, skull.from, damage_type, cause, false);
                // A totem that fired saves the player inside hurtServer:
                // still alive, so run the enchantments instead of healing the owner.
                let killed = self.common.player.as_ref().is_some_and(|pl| {
                    killed(
                        hit,
                        pl.health.dead_or_dying(),
                        pl.health.has_pending_protection(),
                    )
                });
                if killed {
                    self.heal_skull_owner(p, live_owner);
                } else if hit && live_owner.is_some() {
                    self.shot_post_attack(p, Who::Player, damage_type);
                }
                if hit && wither_ticks > 0 {
                    self.give_player_effect("minecraft:wither", wither_ticks, 1);
                }
            }
            // EnderDragonPart.hurtServer / EndCrystal.hurtServer: skull with live owner uses wither_skull, else magic.
            Some(Who::Mob(id)) if self.is_special_target(id) => {
                let damage_type = if live_owner.is_some() {
                    "minecraft:wither_skull"
                } else {
                    "minecraft:magic"
                };
                let hit = |game: &mut Self| {
                    game.hit_special_full(
                        id,
                        amount,
                        live_owner,
                        p.guest,
                        kind,
                        Some(damage_type),
                        Some(super::dragon::DirectHit::from_projectile(p)),
                    );
                };
                if id >= SPECIAL
                    && mod_api::__private::enabled_listened(mod_api::HookId::LivingDeath)
                    && self.server.combat.0.attack().projectile != Some(p.id as i32)
                {
                    let attack = self.projectile_attack(p);
                    self.with_attack(attack, hit);
                } else {
                    hit(self);
                }
            }
            Some(Who::Mob(id)) => {
                let owner_species = owner
                    .and_then(|o| self.common.mobs.iter().find(|m| m.id == o))
                    .map(|m| m.species());
                let source = Damage {
                    kind,
                    attacker: live_owner,
                    from: skull.from,
                    // `magic()` for an ownerless skull names no entity.
                    direct: live_owner.is_some().then(|| p.entity_type()),
                    attacker_species: owner_species,
                };
                let mut survived_hit = false;
                let killed = match self.common.mob_index.find(&self.common.mobs, id) {
                    Some(index) => {
                        let plan = self.prepare_knockback_credit(
                            id as i32,
                            source.attacker,
                            self.server.combat.0.attack(),
                        );
                        // The hit's causing entity, lent to its listeners beside the victim.
                        let causing = self.causer_id(source.attacker);
                        let decision = crate::game::entity_server::causer_decision!(self, causing);
                        let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                        let (victim, lend) = crate::game::entity_server::victim_and_causer!(
                            self, index, causing, decision
                        );
                        let hit = victim.damage(
                            crate::game::entity_server::damage_level_of!(
                                self,
                                self.server.combat.0.attack(),
                                &mut credit,
                                lend
                            ),
                            amount,
                            source,
                        );
                        let Some((index, hit)) = self.complete_mob_hurt(index, hit) else {
                            return Ok(());
                        };
                        self.finish_mob_damage_at(index);
                        let alive = self
                            .common
                            .mobs
                            .get(index)
                            .is_some_and(|m| m.id == id && m.alive());
                        survived_hit = hit && alive;
                        hit && !alive
                    }
                    None => {
                        let mut skull_p = p.clone();
                        skull_p.owner = live_owner;
                        let damage_type = self.projectile_damage_type(&skull_p);
                        let info = sunrise_gameplay_common::game::creatures::ProjectileHitInfo::from_projectile(
                            &skull_p,
                            damage_type,
                            amount,
                            None,
                        );
                        let _ = self.projectile_hurts_non_mob(&info, id);
                        false
                    }
                };
                self.settle_mob_hurt(id);
                if survived_hit && live_owner.is_some() {
                    self.mob_post_attack(
                        id,
                        live_owner,
                        Some((p.id, "minecraft:wither_skull")),
                        Some("minecraft:wither_skull"),
                    );
                }
                if survived_hit && wither_ticks > 0 {
                    let effect_source = match live_owner {
                        Some(Who::Mob(owner_id)) => self.entity(owner_id),
                        Some(Who::Player) => self.entity(self.own_entity_id() as u32),
                        None => None,
                    }
                    .or_else(|| self.entity(p.id));
                    self.add_mob_effect(
                        id,
                        sunrise_level::effects::EffectInstance::new("wither", wither_ticks, 1),
                        effect_source,
                    );
                }
                if killed {
                    self.heal_skull_owner(p, live_owner);
                }
            }
            None => {}
        }
        // Projectile.onHit emits PROJECTILE_LAND after onHitEntity and before WitherSkull.onHit explodes.
        self.projectile_hit_landed_event(p.id, p.owner, hit);
        self.server.combat.0.explosion_owner = owner;
        self.server.combat.0.explosion_in_water = p.touching_water;
        // The blast's source names a player owner, for the dragon's kill credit.
        self.set_blast_by_player(live_owner == Some(Who::Player));
        // WitherSkull.getBlockExplosionResistance: only a blue skull's blast
        // caps a destroyable block's resistance at 0.8 (`isDangerous`).
        if matches!(
            p.kind,
            sunrise_sim::projectiles::ProjectileKind::WitherSkull { dangerous: true }
        ) {
            self.server.combat.0.blast_resistance_cap = Some(0.8);
        }
        self.server.combat.0.explosion_direct = Some((p.id, p.entity_type()));
        let result = self.with_impacting_projectile(p, |g| {
            g.with_hive_emptying_blast(|g| {
                g.explode(
                    Some(p.id),
                    at,
                    1.0,
                    super::explosion::ExplosionSource::default()
                        .with_silence(g.entity_silent(p.id))
                        .with_entity(
                            p.id,
                            "minecraft:wither_skull",
                            sunrise_sim::health::DirectEntity::Projectile,
                        ),
                )
            })
        });
        self.server.combat.0.explosion_direct = None;
        self.server.combat.0.blast_resistance_cap = None;
        self.set_blast_by_player(false);
        self.server.combat.0.explosion_owner = None;
        self.server.combat.0.explosion_in_water = false;
        result
    }
    /// Runs `blast` with the impacting projectile `p` back among the level's
    /// projectiles, as vanilla's skull is still in the level while it
    /// explodes: the tick removed its row before the hit, so a part asked
    /// during the blast (`can_entity_destroy` with the skull) could not read
    /// it. The row returns under its own id and join (`push` keeps a numbered
    /// join), and leaves again afterwards.
    fn with_impacting_projectile<R>(
        &mut self,
        p: &Projectile,
        blast: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.common.creatures.projectiles.push(p.clone());
        let result = blast(self);
        let row = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|q| q.id == p.id && q.join == p.join);
        if let Some(row) = row {
            self.common.creatures.projectiles.remove(row);
        }
        result
    }
    /// `livingOwner.heal(5.0F)` after the skull killed: `LivingEntity.heal`
    /// does nothing to an owner at no health.
    fn heal_skull_owner(&mut self, p: &Projectile, owner: Option<Who>) {
        match owner {
            Some(Who::Mob(id)) => {
                if let Some(w) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && m.alive())
                {
                    w.heal(
                        &self.common.loop_context,
                        &|| crate::game::entity_server::hit_reads_of!(self),
                        5.0,
                    );
                }
            }
            // The player (or the guest acting now) who owns the skull.
            Some(Who::Player) if p.guest.is_none() || p.guest == self.server.lan_acting => {
                self.apply_effect_action(sunrise_level::effects::EffectAction::Heal(5.0));
            }
            _ => {}
        }
    }
}
/// `!target.isAlive()` after a hit that landed: a totem that fired keeps the
/// player alive.
pub(in crate::game::server_side) fn killed(
    hurt: bool,
    dead_or_dying: bool,
    totem_used: bool,
) -> bool {
    hurt && dead_or_dying && !totem_used
}
/// `WitherSkull.onHitEntity`'s damage source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::game) struct SkullSource {
    pub(in crate::game) amount: f32,
    pub(in crate::game) kind: DamageKind,
    /// The source's position: the skull's for `witherSkull(skull, owner)`,
    /// none for `magic()`, which has no entity.
    pub(in crate::game) from: Option<[f64; 3]>,
}
/// 8 `wither_skull` from a skull with a living owner, else 5 `magic`.
/// `skull` is where the skull is.
pub(in crate::game::server_side) fn skull_source(
    owner: Option<Who>,
    skull: [f64; 3],
) -> SkullSource {
    match owner {
        Some(_) => SkullSource {
            amount: 8.0,
            kind: DamageKind::WitherSkull,
            from: Some(skull),
        },
        None => SkullSource {
            amount: 5.0,
            kind: DamageKind::Magic,
            from: None,
        },
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unowned_skull_hurts_as_magic_from_nowhere() {
        // DamageSources.magic(): no entity and no position, so nothing to
        // blame or push away from.
        let at = [1.0, 64.0, 2.0];
        let unowned = skull_source(None, at);
        assert_eq!((unowned.amount, unowned.kind), (5.0, DamageKind::Magic));
        assert_eq!(unowned.from, None);
        // witherSkull(skull, owner): from where the skull is.
        let owned = skull_source(Some(Who::Mob(3)), at);
        assert_eq!((owned.amount, owned.kind), (8.0, DamageKind::WitherSkull));
        assert_eq!(owned.from, Some(at));
        assert_eq!(skull_source(Some(Who::Player), at), owned);
    }

    #[test]
    fn a_totem_that_fires_is_no_kill() {
        assert!(killed(true, true, false));
        // The totem restores health next tick: alive inside hurtServer.
        assert!(!killed(true, true, true));
        assert!(!killed(true, false, false));
        assert!(!killed(false, true, false));
    }
}
