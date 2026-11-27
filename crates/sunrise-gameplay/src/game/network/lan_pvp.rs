//! Players hitting players on a server (`Player.attack` on a player). The hit
//! is worked out as the attacker (weapon, attack strength, critical hit,
//! enchantments, a mace's smash) and lands on the victim like any player
//! attack (blocking, armour, knockback), when the `pvp` game rule allows it
//! (`ServerPlayer.canHarmPlayer`, `isPvpAllowed`). A player's arrow on
//! another player lands the same way.
use super::{
    chat_common::PLAYER_NAME, commands_common::Target, lan_actions_server::CombatPlayer,
    lan_mobs::guest_bounds, mace::Spared,
};

use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_game_data::{Result, text::Text};
use sunrise_sim::{
    health::HurtSource,
    mobs::Who,
    projectiles::{Hit, Projectile, ProjectileKind},
};

/// The player projectiles that meet other players: arrows (their hurt), and
/// snowballs, eggs and mods' thrown types, which break on them
/// (`Player.hurtServer` takes no 0 damage). Pearls, potions and wind charges
/// fly past players as before.
pub(in crate::game) fn meets_players(kind: &ProjectileKind) -> bool {
    matches!(
        kind,
        ProjectileKind::Arrow { .. }
            | ProjectileKind::Snowball
            | ProjectileKind::Egg
            | ProjectileKind::Thrown(_)
    )
}

/// Who a player's attack lands on: a player by connection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Victim {
    Guest(u32),
}

/// The attacker's side of the hit.
pub struct Strike {
    /// The attacking guest, kept after their turn through `as_guest` ends.
    attacker: Option<u32>,
    /// The attacker's creative mode: their hit passes the victim's
    /// invulnerability (`DamageSource.isCreativePlayer`, #916).
    creative: bool,
    pub amount: f32,
    scale: f32,
    crit: bool,
    /// `Player.attack`'s `magicBoost > 0`: the enchantments added damage.
    magic: bool,
    from: [f64; 3],
    /// `getSecondsToDisableBlocking` of the attacker's weapon.
    disable_blocking: f32,
    sweep: Option<[f64; 3]>,
    /// `causeExtraKnockback`: its strength and direction (the attacker's
    /// yaw), and whether the hit is a sprinting knockback attack.
    knockback: (f32, f64, f64),
    knockback_attack: bool,
    direct_motion: Option<(f64, bool)>,
    /// The attacker's team, for the victim's `Player.canHarmPlayer`.
    pub team: Option<sunrise_game_data::scoreboard::Team>,
    /// The attacker's main-hand item with a custom name, for the death
    /// message's `.item` form.
    item: Option<sunrise_game_data::text::Text>,
    /// The weapon that actually produced the strike, before either guest moves.
    pub(in crate::game::server_side) weapon: Option<sunrise_items::items::ItemStack>,
    pub damage_type: String,
}

/// A player's blow on another player as the native hooks name it: the hit's
/// own push and its damage name no attacker yet, so the extra and the sweep's
/// pushes don't either.
const PVP_SOURCE: HurtSource<'static> = HurtSource {
    damage_type: "minecraft:player_attack",
    causing: 0,
    direct: 0,
    position: None,
    direct_type: None,
};

#[cfg(any(test, feature = "test-support"))]
impl Strike {
    /// A plain blow by guest `attacker`, for the tests that land one.
    pub fn test_blow(attacker: u32) -> Self {
        Self {
            attacker: Some(attacker),
            creative: false,
            amount: 1.0,
            scale: 1.0,
            crit: false,
            magic: false,
            from: [3.0, 64.0, 5.0],
            disable_blocking: 0.0,
            sweep: None,
            knockback: (1.0, 1.0, 0.0),
            knockback_attack: false,
            direct_motion: None,
            team: None,
            item: None,
            weapon: None,
            damage_type: "minecraft:player_attack".into(),
        }
    }
}

/// The attacker's pre-hit inputs retained until a sweep's actual victim is in place.
#[derive(Clone)]
pub struct SweepDamage {
    weapon: sunrise_items::items::ItemStack,
    attacker: u32,
    base: f32,
    scale: f32,
    damage_type: String,
    direct_motion: Option<(f64, bool)>,
}

/// A sweep attack's reach to the other players (`Player.doSweepAttack`),
/// worked out as the attacker.
#[derive(Clone)]
pub struct Sweep {
    /// The attacking guest (none: the host).
    pub attacker: Option<u32>,
    /// The attacker's creative mode: their hit passes the victim's
    /// invulnerability (`DamageSource.isCreativePlayer`, #916).
    pub creative: bool,
    pub from: [f64; 3],
    /// The damage after the enchantments and the attack strength.
    pub damage: f32,
    /// Custom effects need the actual victim, so keep their pre-hit input until each landing.
    pub enchanted_damage: Option<SweepDamage>,
    /// The push along the attacker's yaw.
    pub knockback: (f64, f64),
    pub team: Option<sunrise_game_data::scoreboard::Team>,
    /// The target's box grown by (1, 0.25, 1).
    pub hit_box: Aabb,
    /// The entity id of the player the sweep started from, which it spares.
    pub skip: Option<i32>,
}

impl crate::game::ServerGame<'_> {
    /// Whether the player a hit landed on stands on the ground.
    pub(in crate::game::server_side) fn pvp_victim_on_ground(&self, victim: Victim) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        match victim {
            Victim::Guest(conn) => host
                .hub
                .guest(conn)
                .and_then(|g| g.motion.as_deref())
                .is_some_and(|p| p.ground.on_ground),
        }
    }

    /// The player `id` that guest `conn` attacks, if it is one.
    pub(in crate::game::server_side) fn pvp_victim(&self, conn: u32, id: i32) -> Option<Victim> {
        let host = self.server.lan.as_ref()?;
        host.hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g) && !g.dead)
            .map(|g| Victim::Guest(g.conn))
    }

    /// `Player.attack`'s `causeFoodExhaustion(0.1F)`, charged to the attacker
    /// (the host, or a guest swapped in) only once the hit hurt the victim.
    pub(in crate::game::server_side) fn pvp_exhaustion(&mut self) {
        if let Some(p) = self.common.player.as_mut()
            && !self.common.creative
        {
            p.cause_food_exhaustion(0.1);
        }
    }

    /// The attacker's side of `Player.attack` (the host, or a guest swapped
    /// in): the damage, the charge reset and the swing.
    pub(in crate::game::server_side) fn pvp_strike(&mut self, victim: u32) -> Option<Strike> {
        let p = self.common.player.as_ref()?;
        let from = [p.position.x, p.position.y, p.position.z];
        let (amount, scale, crit, magic) =
            self.melee_damage("minecraft:player", true, Some(victim));
        let sweep = self.sweep_attack_position(scale, crit, "minecraft:player");
        let strength = self.player_attack_knockback();
        let knockback_attack =
            scale > 0.9 && self.common.player.as_ref().is_some_and(|p| p.sprinting);
        let (xd, zd) = self.extra_knockback_direction();
        self.server.attack_ticks = 0;
        self.swing_arm();
        // LivingEntity.getSecondsToDisableBlocking: the held weapon's value,
        // none while another item is in use.
        let disable_blocking = if self.common.using.is_some() {
            0.0
        } else {
            self.common
                .held()
                .map_or(0.0, |stack| self.items().weapon_disable_blocking(stack))
        };
        let weapon = self.held().cloned();
        let motion = self
            .player
            .as_ref()
            .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying));
        let damage_type = self.stack_melee_damage_type(weapon.as_ref(), motion, true);
        Some(Strike {
            attacker: self.server.lan_acting,
            creative: self.common.creative,
            amount,
            scale,
            crit,
            magic,
            from,
            disable_blocking,
            sweep,
            knockback: (strength, xd, zd),
            knockback_attack,
            direct_motion: Some(motion),
            team: self.player_team().cloned(),
            item: self.main_hand_custom_name(),
            weapon,
            damage_type,
        })
    }

    /// The guests by connection, with their entity ids.
    pub(in crate::game::server_side) fn lan_guest_entities(&self) -> Vec<(u32, u32)> {
        self.server.lan.as_ref().map_or_else(Vec::new, |host| {
            host.hub
                .guests
                .iter()
                .map(|g| (g.conn, g.entity_id as u32))
                .collect()
        })
    }

    /// The player a player's projectile hit, when it hit one.
    pub(in crate::game) fn projectile_pvp_victim(
        &self,
        p: &Projectile,
        hit: Hit,
    ) -> Option<Victim> {
        if p.owner != Some(Who::Player) || !meets_players(&p.kind) {
            return None;
        }
        let Hit::Entity { who, .. } = hit else {
            return None;
        };
        match who {
            Who::Player => None,
            Who::Mob(id) => self
                .server
                .lan
                .as_ref()?
                .hub
                .guests
                .iter()
                .find(|g| g.entity_id as u32 == id)
                .map(|g| Victim::Guest(g.conn)),
        }
    }

    /// `ServerPlayer.canHarmPlayer` of the players `attacker` and `victim` (entity ids) on
    /// their teams: the `pvp` game rule as it stands, handed to
    /// [`can_harm_player`](sunrise_client::health::can_harm_player), which decides.
    pub(in crate::game::server_side) fn player_may_harm(
        &self,
        attacker: i32,
        victim: i32,
        attacker_team: Option<&sunrise_game_data::scoreboard::Team>,
        victim_team: Option<&sunrise_game_data::scoreboard::Team>,
    ) -> bool {
        let answer = sunrise_sim::health::can_harm_player(
            attacker,
            victim,
            self.common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::PVP),
            attacker_team,
            victim_team,
        );
        #[cfg(any(test, feature = "test-support"))]
        let answer = sunrise_gameplay_common::game::rule_replacements::harm(attacker, victim)
            .unwrap_or(answer);
        let _ = (attacker, victim);
        answer
    }

    /// [`player_may_harm`](Self::player_may_harm) for a hit on the guest `victim` by the guest
    /// `attacker` (connections), or by the player in place when none: a player's projectile or
    /// trident, which the victim's hurt refuses when it answers false. A guest no longer here
    /// is entity -1, on no team.
    pub fn guest_may_harm(&self, attacker: Option<u32>, victim: u32) -> bool {
        let board = &self.common.score.board;
        let guest = |conn: u32| {
            self.server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .map_or((-1, None), |g| (g.entity_id, board.players_team(&g.name)))
        };
        let (attacker, attacker_team) = match attacker {
            Some(conn) => guest(conn),
            None => (self.own_entity_id(), self.player_team()),
        };
        let (victim, victim_team) = guest(victim);
        self.player_may_harm(attacker, victim, attacker_team, victim_team)
    }

    /// A player's projectile on a player: the hit runs as the victim, whose
    /// hurt refuses it when the `pvp` game rule or their teams forbid it
    /// (`ServerPlayer.canHarmPlayer`; `AbstractArrow.onHitEntity` then turns
    /// the arrow back).
    pub fn projectile_hits_player(
        &mut self,
        p: &Projectile,
        hit: Hit,
        victim: Victim,
    ) -> Result<()> {
        let Hit::Entity { at, .. } = hit else {
            return Ok(());
        };
        if self.server.lan.is_none() {
            return self.projectile_hit(p, hit);
        }
        let refused = match victim {
            Victim::Guest(conn) => !self.guest_may_harm(p.guest, conn),
        };
        let on_player = Hit::Entity {
            who: Who::Player,
            at,
        };
        let shooter = self.player_attacker(p.guest);
        let effect_attacker = self.player_entity_id_by_connection(p.guest);
        let Victim::Guest(conn) = victim;
        let mut landed_arrow = None;
        self.guest_hit_by(conn, |g| {
            g.server.combat.0.refuse_player_hit = refused;
            g.server.combat.0.projectile_attacker = shooter;
            let saved_effect_attacker = std::mem::replace(
                &mut g.server.combat.0.enchantment_projectile_attacker,
                effect_attacker,
            );
            let displaced = effect_attacker.is_some_and(|id| {
                matches!(
                    g.combat_player(id as u32),
                    Some(CombatPlayer::Displaced(..))
                )
            });
            let result = g.projectile_hit_deferring_player_arrow(
                p,
                on_player,
                displaced.then_some(&mut landed_arrow),
            );
            g.server.combat.0.refuse_player_hit = false;
            g.server.combat.0.projectile_attacker = None;
            g.server.combat.0.enchantment_projectile_attacker = saved_effect_attacker;
            result
        })
        .unwrap_or(Ok(()))?;
        if let Some(landed) = landed_arrow {
            self.resume_guest_arrow_hit(conn, landed);
        }
        Ok(())
    }

    /// `AbstractArrow.onHitEntity`: an arrow that hurt a player tells its owner, a player other
    /// than the one hit, with `PLAY_ARROW_HIT_SOUND`, unless the arrow is silent.
    pub(in crate::game::server_side) fn arrow_hit_sound_to_shooter(&self, p: &Projectile) {
        let (Some(host), Some(shooter)) = (self.server.lan.as_ref(), p.guest) else {
            return;
        };
        if self.server.lan_acting == Some(shooter) || self.entity_silent(p.id) {
            return;
        }
        host.hub.send_to(
            shooter,
            &sunrise_protocol::packets::play::Clientbound::GameEvent(
                sunrise_protocol::packets::play::GameEvent {
                    event: sunrise_sim::lan::hub::game_event::PLAY_ARROW_HIT_SOUND,
                    param: 0.0,
                },
            ),
        );
    }

    pub(in crate::game::server_side) fn player_attacker(
        &self,
        conn: Option<u32>,
    ) -> Option<(u32, Text)> {
        match conn {
            Some(conn) => {
                let g = self.server.lan.as_ref()?.hub.guest(conn)?;
                Some((g.entity_id as u32, self.common.player_text(&g.name, g.uuid)))
            }
            None => None,
        }
    }

    /// The hit landing on the victim with its extra knockback, then the
    /// attacker's attack sound. Returns whether extra knockback was applied when hurt.
    pub fn pvp_land(&mut self, victim: Victim, s: &Strike) -> Option<bool> {
        self.server.lan.as_ref()?;
        if s.knockback_attack {
            self.server_play_sound("minecraft:entity.player.attack.knockback", s.from, 1.0, 1.0);
        }
        let host = self.server.lan.as_ref()?;
        // CombatTracker: the attacking player is the death message's killer.
        // Without it the message is built with one argument and a template
        // that names a killer ("%1$s was slain by %2$s") shows as written.
        let attacker_note = s.attacker.and_then(|conn| {
            host.hub
                .guest(conn)
                .map(|a| (a.entity_id, self.common.player_text(&a.name, a.uuid)))
        });
        let attacker_id = self.common.player.as_ref().map_or(0, |p| p.entity_id);
        let attacker_id = attacker_note.as_ref().map_or(attacker_id, |(id, _)| *id);
        // The blow's attacker as `canHarmPlayer` asks it: the host with no attacking guest,
        // and a guest no longer here as entity -1 (with the blow's own team), as
        // `guest_may_harm` asks it.
        let harm_attacker = match (s.attacker, &attacker_note) {
            (Some(_), Some((id, _))) => *id,
            (Some(_), None) => -1,
            (None, _) => attacker_id,
        };
        let (victim_id, team) = match victim {
            Victim::Guest(conn) => host.hub.guest(conn).map_or((-1, None), |g| {
                (g.entity_id, self.common.score.board.players_team(&g.name))
            }),
        };
        let harm = self.player_may_harm(harm_attacker, victim_id, s.team.as_ref(), team);
        // Player.attack: the hurt (with its knockback away from the
        // attacker), then causeExtraKnockback along the attacker's yaw.
        // Taken here, before the victim is swapped in, so the attacker's hands are found.
        let attack = self.attack_of(attacker_id);
        let land = |g: &mut Self| {
            g.with_attack(attack.clone(), |g| {
                g.server.combat.0.player_attacker = Some((s.from, s.disable_blocking));
                if let Some((id, name)) = attacker_note.clone() {
                    g.note_player_attacker(&s.damage_type, id as u32, name, s.item.clone());
                }
                let hurt = g.mob_hits_player_from(s.amount, s.from, &s.damage_type, s.creative);
                g.server.combat.0.player_attacker = None;
                if !hurt {
                    return None;
                }
                let (base, xd, zd) = s.knockback;
                let source = attack.player_source(&s.damage_type, attack.attacker);
                let mut ctx = attack.enchantment_context(source, "minecraft:player");
                ctx.direct_motion = s.direct_motion;
                ctx.attacker_motion = s.direct_motion;
                let strength = s.weapon.as_ref().map_or(base, |weapon| {
                    g.evaluate_weapon_value(
                        weapon,
                        "minecraft:knockback",
                        Some(g.own_entity_id() as u32),
                        ctx,
                        base,
                    )
                });
                let strength = strength * 0.5 + if s.knockback_attack { 0.5 } else { 0.0 };
                if strength > 0.0 {
                    g.knock_back_player_full(
                        f64::from(strength),
                        xd,
                        zd,
                        PVP_SOURCE,
                        &attack,
                        s.amount,
                        true,
                    );
                }
                Some(strength > 0.0)
            })
        };
        let landed = harm
            .then(|| match victim {
                Victim::Guest(conn) => self.guest_struck_by(conn, land).flatten(),
            })
            .flatten();
        let hurt = landed.is_some();
        if hurt && (s.crit || s.magic) {
            let target = match victim {
                Victim::Guest(conn) => self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .and_then(|g| g.motion.as_deref().map(|p| (g.entity_id, p))),
            };
            if let Some((id, p)) = target {
                let d = p.dimensions;
                let (feet, width, height) = (
                    [p.position.x, p.position.y, p.position.z],
                    f64::from(d.width),
                    f64::from(d.height),
                );
                if s.crit {
                    self.critical_hit(s.attacker, id, feet, width, height, false);
                }
                if s.magic {
                    self.critical_hit(s.attacker, id, feet, width, height, true);
                }
            }
        }
        let sound = match (hurt, s.crit, s.scale > 0.9) {
            (false, ..) => "minecraft:entity.player.attack.nodamage",
            (true, true, _) => "minecraft:entity.player.attack.crit",
            (true, false, true) => "minecraft:entity.player.attack.strong",
            (true, false, false) => "minecraft:entity.player.attack.weak",
        };
        let swept = s.sweep.filter(|_| hurt);
        if let Some(at) = swept {
            // The sweep around the player that was hit, as its attacker.
            let (area, id) = match victim {
                Victim::Guest(conn) => {
                    let g = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn));
                    (g.map(guest_bounds), g.map_or(-1, |g| g.entity_id))
                }
            };
            let target = area.map(|area| (area, Spared::Player(id)));
            match s.attacker {
                None => {
                    // The host is checked out here: collect the sweep for the players.
                    self.server.combat.0.sweeps = Some(Vec::new());
                    self.sweep_attack(at, Some(s.from), target, s.scale);
                    for sweep in self.server.combat.0.sweeps.take().unwrap_or_default() {
                        self.sweep_players(&sweep);
                    }
                }
                Some(conn) => {
                    self.as_guest(conn, |g| g.sweep_attack(at, Some(s.from), target, s.scale));
                }
            }
        }
        // A critical hit a mod let sweep has both sounds (attackVisualEffects).
        if swept.is_none() || s.crit {
            self.server_play_sound(sound, s.from, 1.0, 1.0);
        }
        landed
    }

    /// `ServerPlayer.die`'s `killer.awardKillScore(this, source)` for a guest,
    /// whatever source killed them (`LivingEntity.getKillCredit`), then
    /// `createWitherRose(killer)`: the killer's kill counts and `player_kills`,
    /// and the rose a wither's kill leaves where they fell. A mob credits no
    /// score.
    pub(in crate::game::server_side) fn guest_award_kill_score(
        &mut self,
        victim: u32,
        at: [f64; 3],
    ) {
        let Some(id) = self
            .guest_combat_record(victim)
            .and_then(|c| c.kill_credit(self.common.world.game_time))
        else {
            return;
        };
        // `awardKillScore` excludes the victim, even when that body is in place.
        if let Some(killer) = self.combat_player(id)
            && let Some(name) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(victim))
                .filter(|g| g.entity_id as u32 != id)
                .map(|g| g.name.clone())
        {
            match killer {
                CombatPlayer::InPlace => {
                    self.award_custom_stat("player_kills", 1);
                    self.score_player_killed_named(Some(name), true);
                }
                CombatPlayer::Displaced(depth, _) => {
                    let frame = &self.server.lan_displaced_players.0[depth];
                    let killer = frame.name.as_deref().unwrap_or(PLAYER_NAME).to_owned();
                    self.award_displaced_stat(depth, "player_kills", 1);
                    self.score_player_kill_for(&killer, Some(&name), true);
                }
                CombatPlayer::Parked(guest) => {
                    self.as_guest(guest.conn, |g| {
                        g.award_custom_stat("player_kills", 1);
                        g.score_player_killed_named(Some(name), true);
                    });
                }
            }
        }
        self.create_wither_rose_by(at, Some(id));
    }

    /// The sweep of the player in place, for the players it reaches: the
    /// damage (`1 + ratio * baseDamage`, plus the enchantments against a
    /// player, scaled by the attack strength) and who hits. The box is set
    /// by the caller.
    pub(in crate::game::server_side) fn player_sweep(&mut self, base: f32, scale: f32) -> Sweep {
        let from = self
            .common
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let enchanted_damage =
            self.common
                .held()
                .zip(self.common.player.as_ref())
                .map(|(held, player)| {
                    let weapon = self.common.item_combat_stack(held);
                    let motion = (player.fall_distance, player.fall_flying);
                    let damage_type =
                        self.common
                            .stack_melee_damage_type(Some(&weapon), motion, true);
                    SweepDamage {
                        weapon,
                        attacker: player.entity_id as u32,
                        base,
                        scale,
                        damage_type,
                        direct_motion: Some(motion),
                    }
                });
        let damage = if enchanted_damage.is_some() {
            0.0
        } else {
            base * scale
        };
        Sweep {
            attacker: self.server.lan_acting,
            creative: self.common.creative,
            from,
            damage,
            enchanted_damage,
            knockback: self.extra_knockback_direction(),
            team: self.player_team().cloned(),
            hit_box: Aabb::new(Vec3::ZERO, Vec3::ZERO),
            skip: None,
        }
    }

    /// A sweep from a mob hit reaches the other players. Inside a guest's
    /// turn the host holds the other players (`as_guest`), so it waits for
    /// the turn to end.
    pub(in crate::game::server_side) fn sweep_other_players(&mut self, sweep: Sweep) {
        if let Some(collect) = self.server.combat.0.sweeps.as_mut() {
            collect.push(sweep);
        } else if self.server.lan_acting.is_some() {
            self.server.sweeps.push(sweep);
        } else {
            self.sweep_players(&sweep);
        }
    }

    /// The sweeps guests' turns put off.
    pub(in crate::game::server_side) fn flush_sweeps(&mut self) {
        for sweep in std::mem::take(&mut self.server.sweeps) {
            self.sweep_other_players(sweep);
        }
    }

    /// `Player.doSweepAttack` on the players: each other player in the box,
    /// closer than 3 blocks (`distanceToSqr < 9`) and not an ally
    /// (`isAlliedTo`), is hurt (`ServerPlayer.canHarmPlayer`: the `pvp` rule)
    /// and pushed 0.4 along the attacker's yaw.
    pub fn sweep_players(&mut self, sweep: &Sweep) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let from = Vec3::new(sweep.from[0], sweep.from[1], sweep.from[2]);
        let mut victims = Vec::new();
        for g in &host.hub.guests {
            if host.hub.here(g)
                && g.loaded
                && !g.dead
                && !g.spectator()
                && sweep.attacker != Some(g.conn)
                && guest_bounds(g).intersects(sweep.hit_box)
                && Vec3::new(g.position[0], g.position[1], g.position[2]).distance_squared(from)
                    < 9.0
            {
                let team = self.common.score.board.players_team(&g.name).cloned();
                victims.push((Victim::Guest(g.conn), g.entity_id, team));
            }
        }
        let (xd, zd) = sweep.knockback;
        // The sweep uses the attacking player's source, even for a local body.
        let attacker = match sweep.attacker {
            Some(conn) => self.player_attacker(Some(conn)),
            None => self
                .common
                .player
                .as_ref()
                .map(|p| (p.entity_id as u32, self.target_name(Target::Player))),
        };
        let attacker_id = self.common.player.as_ref().map_or(0, |p| p.entity_id);
        let attack = self.attack_of(attacker.as_ref().map_or(attacker_id, |(id, _)| *id as i32));
        let land = |g: &mut Self| {
            let damage = if let Some(damage) = &sweep.enchanted_damage {
                let victim = g.player.as_ref().map(|player| player.entity_id as u32);
                g.weapon_enchanted_damage(
                    &damage.weapon,
                    victim.map(|victim| (damage.attacker, victim)),
                    "minecraft:player",
                    &damage.damage_type,
                    damage.base,
                    damage.direct_motion,
                ) * damage.scale
            } else {
                sweep.damage
            };
            let damage_type = sweep
                .enchanted_damage
                .as_ref()
                .map_or("minecraft:player_attack", |damage| {
                    damage.damage_type.as_str()
                });
            g.with_attack(attack.clone(), |g| {
                if let Some((id, name)) = &attacker {
                    g.note_named_attacker(damage_type, *id, name.clone());
                }
                let hurt = g.mob_hits_player_from(damage, sweep.from, damage_type, sweep.creative);
                if hurt {
                    g.knock_back_player(0.4, xd, zd, PVP_SOURCE, &attack, damage);
                }
                hurt
            })
        };
        let attacker_id = attacker.as_ref().map_or(attacker_id, |(id, _)| *id as i32);
        for (victim, id, team) in victims {
            if sweep.skip == Some(id)
                || sunrise_game_data::scoreboard::teams_allied(team.as_ref(), sweep.team.as_ref())
                || !self.player_may_harm(attacker_id, id, sweep.team.as_ref(), team.as_ref())
            {
                continue;
            }
            let Victim::Guest(conn) = victim;
            if self.guest_struck_by(conn, land).unwrap_or_default() {
                let damage_type = sweep
                    .enchanted_damage
                    .as_ref()
                    .map_or("minecraft:player_attack", |damage| {
                        damage.damage_type.as_str()
                    });
                let source = attack.player_source(damage_type, attack.attacker);
                let weapon = sweep.enchanted_damage.as_ref().map(|damage| &damage.weapon);
                self.post_attack(
                    id as u32,
                    "minecraft:player",
                    &attack,
                    source,
                    weapon,
                    false,
                );
            }
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
