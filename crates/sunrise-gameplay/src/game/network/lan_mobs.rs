//! Mobs and the guests of an open world. The mob code sees one player
//! (`MobWorld.player`, `Who::Player`); with guests on, each mob sees the
//! player vanilla's `Level.getNearestPlayer` would give it, and keeps the
//! guest it targets until it lets go. What it then does to `Who::Player`
//! (hits, pushes, fire, its projectiles) runs with that guest swapped in.
use crate::game::ServerGame;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay::lan::mob_players::guest_view;
pub(in crate::game) use sunrise_gameplay::lan::mob_players::{
    Present, guest_bounds, omen_of, player_for,
};
use sunrise_gameplay_common::lan::mob_players::GuestViews;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::{
    mobs::{MobEvent, Who},
    projectiles::{Hit, Projectile},
};

impl ServerGame<'_> {
    /// How mobs see each guest in play (`player_view` with the guest swapped
    /// in).
    pub fn lan_guest_views(&mut self) -> GuestViews {
        let Some(host) = self.server.lan.as_ref() else {
            return Vec::new();
        };
        let conns: Vec<(u32, u128, [f64; 3])> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.is_fake())
            .map(|g| {
                let moved = [0, 1, 2].map(|k| g.position[k] - g.previous_position[k]);
                (g.conn, g.uuid, moved)
            })
            .collect();
        let mut views = Vec::new();
        for (conn, uuid, moved) in conns {
            if let Some(Some(v)) = self.as_guest(conn, |g| g.player_view()) {
                views.push((conn, guest_view(v, uuid, moved)));
            }
        }
        views
    }

    /// The guest `conn`'s real identity — the uuid its lent view carries and
    /// its entity id — when the guest is a player the tick can name: a fake
    /// guest resolves as none, as the write handle refuses it (#1077).
    pub fn guest_identity(&self, conn: u32) -> Option<([i32; 4], u32)> {
        let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
        if guest.is_fake() {
            return None;
        }
        let uuid = sunrise_sim::lan::playerdata::uuid_ints(guest.uuid);
        let id = u32::try_from(guest.entity_id).ok()?;
        Some((uuid, id))
    }

    /// Runs `f` as each guest in play here, for what strikes every player in
    /// an area (`Level.getEntities`), not only the window's own.
    pub(in crate::game::server_side) fn each_guest_hit(&mut self, f: impl FnMut(&mut Self)) {
        self.each_guest_hit_except(None, f);
    }

    /// [`Self::each_guest_hit`] leaving out the guest `except` (the one a
    /// firework rocket is attached to, which it hurts on its own).
    pub(in crate::game::server_side) fn each_guest_hit_except(
        &mut self,
        except: Option<u32>,
        mut f: impl FnMut(&mut Self),
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && Some(g.conn) != except)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            self.guest_hit_by(conn, &mut f);
        }
    }

    /// Mob events aimed at a guest, run as that guest (`guest_hit_by`).
    pub(in crate::game) fn lan_guest_mob_events(&mut self, events: &mut Vec<(u32, u32, MobEvent)>) {
        if events.is_empty() {
            return;
        }
        for (conn, mob, e) in
            sunrise_gameplay_common::game::creatures::RemainingActions::new(events)
        {
            self.guest_hit_by(conn, |g| g.guest_mob_event(mob, e));
        }
    }

    /// One mob event, with the guest swapped in. A hit knows the mob that
    /// swung, so nothing guesses it from where the hit came from.
    pub fn guest_mob_event(&mut self, mob: u32, e: MobEvent) {
        if matches!(
            e,
            MobEvent::AttackPlayer(..)
                | MobEvent::AttackPlayerWithEffect { .. }
                | MobEvent::Sting {
                    target: Who::Player,
                    ..
                }
        ) {
            self.server.combat.0.melee_mob = Some(mob);
        }
        let _ = self.handle_mob_events(vec![e]);
        // Used up by the hit; a blow that hurt nobody leaves it to no other.
        self.server.combat.0.melee_mob = None;
    }

    /// The guests' boxes for the blocks entities trigger (pressure plates,
    /// tripwires, detector rails): `Entity.checkInsideBlocks` and
    /// `Level.getEntities` see every player in the level, but not spectators
    /// (`Entity.isIgnoringBlockTriggers`).
    pub(in crate::game) fn lan_block_trigger_boxes(&self) -> Vec<([f64; 3], [f64; 3])> {
        let Some(host) = self.server.lan.as_ref() else {
            return Vec::new();
        };
        host.hub
            .guests
            .simulation_iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.spectator())
            .map(|g| {
                let b = g.bounds();
                let (lo, hi) = (b.min_position(), b.max_position());
                ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
            })
            .collect()
    }
    /// The same guests' cast fishing hooks, for `GameLevel::trigger_projectiles`.
    pub(in crate::game::server_side) fn lan_hook_triggers(&self, out: &mut Vec<(Aabb, bool)>) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        out.extend(
            host.hub
                .guests
                .simulation_iter()
                .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.spectator())
                .filter_map(|g| host.play.get(&g.conn)?.hook.as_ref())
                .map(|hook| (hook.bounds(), hook.applies_block_effects())),
        );
    }
    /// The same guests' feet and whether they stand on the ground, for the
    /// blocks that also ask that (`BigDripleafBlock.canEntityTilt`).
    pub(in crate::game::server_side) fn lan_guest_feet(&self) -> Vec<([f64; 3], bool)> {
        let Some(host) = self.server.lan.as_ref() else {
            return Vec::new();
        };
        host.hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.spectator())
            .map(|g| (g.position, g.on_ground))
            .collect()
    }

    /// A projectile that hit a guest: its hit runs as theirs. False when
    /// `id` is not a guest.
    pub(in crate::game::server_side) fn lan_projectile_hit(
        &mut self,
        p: &Projectile,
        id: u32,
        at: Vec3,
    ) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let conn = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id as u32 == id)
            .map(|g| g.conn);
        if let Some(conn) = conn {
            self.guest_hit_by(conn, |g| {
                let hit = Hit::Entity {
                    who: Who::Player,
                    at,
                };
                let _ = g.projectile_hit(p, hit);
            });
        }
        conn.is_some()
    }
}

impl ServerGame<'_> {
    /// Hurt a guest: `f` runs as them, and the knockback it gave reaches
    /// their client. A hurt marks the velocity (Entity.markHurt), which goes
    /// out once the guest's server copy has travelled (`guest_server_tick`);
    /// a push with no hurt goes out at once.
    pub(in crate::game::server_side) fn guest_hit_by<R>(
        &mut self,
        conn: u32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let (result, pushed) = self.as_guest(conn, |g| {
            let before = g.player.as_ref().map(|p| p.velocity);
            let result = f(g);
            let pushed = g
                .player
                .as_ref()
                .filter(|p| !p.health.sync_velocity && Some(p.velocity) != before)
                .map(|p| p.velocity);
            (result, pushed)
        })?;
        self.send_guest_motion(conn, pushed);
        Some(result)
    }

    /// A player's melee hit on a guest (Player.causeExtraKnockback on a
    /// ServerPlayer): a hurt that marked the velocity sends it at once, and
    /// the server's copy goes back to the velocity it had before the hit.
    pub fn guest_struck_by<R>(&mut self, conn: u32, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let (result, sent) = self.as_guest(conn, |g| {
            let before = g.player.as_ref().map(|p| p.velocity);
            let result = f(g);
            let sent = g.player.as_mut().zip(before).and_then(|(p, before)| {
                std::mem::take(&mut p.health.sync_velocity)
                    .then(|| std::mem::replace(&mut p.velocity, before))
            });
            (result, sent)
        })?;
        self.send_guest_motion(conn, sent);
        Some(result)
    }

    /// ClientboundSetEntityMotionPacket to a guest about themselves.
    pub(in crate::game::server_side) fn send_guest_motion(
        &self,
        conn: u32,
        velocity: Option<Vec3>,
    ) {
        if let (Some(v), Some(g)) = (
            velocity,
            self.server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn)),
        ) {
            g.send(&C::SetEntityMotion(play::SetEntityMotion {
                id: g.entity_id,
                velocity: [v.x, v.y, v.z],
            }));
        }
    }
}

impl ServerGame<'_> {
    /// Every guest in the world (`ServerLevel.getPlayers` past the host).
    pub fn lan_present(&self) -> Vec<Present> {
        let guests = self
            .server
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.simulation_iter().filter(|g| h.hub.here(g)));
        guests
            .filter(|g| g.loaded)
            .map(|g| {
                let [x, y, z] = g.position;
                // The acting guest's effects are on the game while their turn runs.
                let effects = if self.server.lan_acting == Some(g.conn) {
                    &self.common.potions.effects
                } else {
                    &g.effects
                };
                Present {
                    uuid: sunrise_sim::lan::playerdata::uuid_ints(g.uuid),
                    position: g.position,
                    eye: [x, y + g.eye_height(), z],
                    height: g.dimensions().height,
                    creative: g.creative(),
                    spectator: g.spectator(),
                    alive: !g.dead,
                    omen: omen_of(effects),
                }
            })
            .collect()
    }
    /// The UUID of the player acting: the guest `as_guest` runs, else the host.
    pub fn acting_uuid(&self) -> [i32; 4] {
        self.server
            .lan_acting_uuid
            .map(sunrise_sim::lan::playerdata::uuid_ints)
            .or(self.common.level.player_uuid)
            .unwrap_or_default()
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_gameplay::lan::mob_players::choose;
    use sunrise_sim::mobs::{Mob, PlayerView};

    #[test]
    fn a_guests_view_keeps_who_hurt_them_and_who_they_hurt() {
        let mut v = view(0.0, false);
        v.hurt_by = Some((Who::Mob(7), 90));
        v.last_hurt = Some((8, 95));
        let seen = guest_view(v, 5, [1.0, 0.0, 0.0]);
        assert_eq!(seen.hurt_by, Some((Who::Mob(7), 90)));
        assert_eq!(seen.last_hurt, Some((8, 95)));
        assert!(seen.remote);
        assert_eq!(seen.velocity, Vec3::new(1.0, 0.0, 0.0));
    }

    fn view(x: f64, creative: bool) -> PlayerView {
        PlayerView {
            position: Vec3::new(x, 64.0, 0.0),
            bounds: Aabb::new(Vec3::new(x, 64.0, 0.0), Vec3::new(x, 65.8, 0.0)),
            eye: [x, 65.62, 0.0],
            alive: true,
            invulnerable: false,
            creative,
            sneaking: false,
            swimming: false,
            look: [0.0; 3],
            main_hand: None,
            off_hand: None,
            head: None,
            chest: None,
            legs: None,
            feet: None,
            armor_cover: 0.0,
            invisible: false,
            spectator: false,
            gold_armor: false,
            hurt_by: None,
            last_hurt: None,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            sprinting: false,
            riding: false,
            hero: false,
            effects: Vec::new(),
            uuid: [0; 4],
            allies: Default::default(),
            remote: false,
            sleeping: false,
            sleep_timer: 0,
            passenger: false,
            riding_vehicle: None,
            vehicle: None,
        }
    }

    const MOB: Vec3 = Vec3::new(0.0, 64.0, 0.0);

    #[test]
    fn the_nearest_survival_player_is_seen() {
        let host = view(10.0, false);
        let guests = vec![(7, view(3.0, false)), (8, view(-5.0, false))];
        assert_eq!(
            choose(MOB, false, false, Some(&host), &guests, None),
            Some(7)
        );
        // A creative guest is passed over for the host further away.
        let guests = vec![(7, view(3.0, true))];
        assert_eq!(choose(MOB, false, false, Some(&host), &guests, None), None);
        // Nobody targetable: the nearest one is still seen.
        let host = view(10.0, true);
        assert_eq!(
            choose(MOB, false, false, Some(&host), &guests, None),
            Some(7)
        );
        assert_eq!(choose(MOB, false, false, None, &Vec::new(), None), None);
    }

    #[test]
    fn a_targeted_guest_stays_the_target() {
        let host = view(1.0, false);
        let guests = vec![(7, view(12.0, false))];
        assert_eq!(
            choose(MOB, true, true, Some(&host), &guests, Some(7)),
            Some(7)
        );
        // Gone creative while only recently hurt by them: the mob looks again.
        let guests = vec![(7, view(12.0, true))];
        assert_eq!(
            choose(MOB, true, false, Some(&host), &guests, Some(7)),
            None
        );
        // Without a target the nearest player wins.
        let guests = vec![(7, view(12.0, false))];
        assert_eq!(
            choose(MOB, false, false, Some(&host), &guests, Some(7)),
            None
        );
    }

    /// A mob's target is one player: one that stops being a valid target is
    /// dropped by the mob's goals, never handed to a farther player.
    #[test]
    fn a_target_that_stops_being_valid_is_not_passed_to_another_player() {
        let guests = vec![(7, view(18.0, false))];
        for (alive, creative, spectator) in [
            (false, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            let mut host = view(2.0, creative);
            host.alive = alive;
            host.spectator = spectator;
            // The host it targets: kept, not swapped for the guest.
            assert_eq!(choose(MOB, true, true, Some(&host), &guests, None), None);
            // A targeted guest in the same state: kept too.
            let mut guest = host.clone();
            guest.position = Vec3::new(18.0, 64.0, 0.0);
            let near = view(2.0, false);
            assert_eq!(
                choose(MOB, true, true, Some(&near), &vec![(7, guest)], Some(7)),
                Some(7)
            );
        }
        // A targeted guest who left: the mob looks again.
        assert_eq!(
            choose(MOB, true, true, Some(&view(2.0, false)), &guests, Some(9)),
            None
        );
    }

    /// #3806 cases 1 to 3: a mob's player target, host or guest, is kept tick
    /// after tick while it stays valid, however near the other players are,
    /// and the guests' order or a guest joining changes nothing. Each tick
    /// feeds the binding back in, as the mob loop does.
    #[test]
    fn a_valid_player_target_is_kept_across_ticks_and_guest_changes() {
        let ticks = |host: &PlayerView, rounds: &[GuestViews], start: Option<u32>| {
            let mut bound = start;
            for guests in rounds {
                bound = choose(MOB, true, true, Some(host), guests, bound);
                let order: Vec<u32> = guests.iter().map(|g| g.0).collect();
                assert_eq!(bound, start, "with guests {order:?}");
            }
        };
        let rounds = |a: f64, b: f64| {
            vec![
                vec![(7, view(a, false)), (8, view(b, false))],
                // The same guests in the other order.
                vec![(8, view(b, false)), (7, view(a, false))],
                // A third guest joins, nearer than anyone.
                vec![
                    (7, view(a, false)),
                    (9, view(0.5, false)),
                    (8, view(b, false)),
                ],
            ]
        };
        // 1. The host it targets, farther than both guests.
        ticks(&view(20.0, false), &rounds(2.0, 3.0), None);
        // 2. A guest it targets, farther than the host and the other guest.
        ticks(&view(2.0, false), &rounds(20.0, 3.0), Some(7));
    }

    #[test]
    fn a_leashed_mob_sees_its_holder_even_when_another_player_is_closer() {
        let Some(mut cow) = mob_of("minecraft:cow") else {
            return;
        };
        let mut holder = view(10.0, true);
        holder.uuid = [1, 2, 3, 4];
        let mut nearby = view(1.0, false);
        nearby.uuid = [5, 6, 7, 8];
        cow.extra.leash = Some(sunrise_sim::mobs::leash::Holder::Player);
        cow.extra.leash_player = Some(holder.uuid);
        let guests = vec![(7, holder.clone()), (8, nearby.clone())];
        assert_eq!(player_for(&cow, None, &guests, None), Some(7));
        assert_eq!(
            player_for(&cow, Some(&holder), &vec![(8, nearby)], None),
            None
        );
    }

    /// The mob of the prepared resources' type `kind` (None without them).
    fn mob_of(kind: &str) -> Option<Mob> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let types = sunrise_sim::mobs::MobTypes::load(&root).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        Some(Mob::new(1, t, MOB, 0.0))
    }

    /// #6781: a golem a guest hit stays angry for 400+ ticks but remembers
    /// its last attacker for 100. After that it must still see the guest it
    /// is angry at, not the bystander standing nearer.
    #[test]
    fn an_angry_golem_keeps_the_player_who_hit_it() {
        let Some(mut golem) = mob_of("minecraft:iron_golem") else {
            return;
        };
        golem.age = 500;
        golem.ai.last_hurt_by = Some(Who::Player);
        golem.ai.last_hurt_by_time = 100;
        golem.ai.target = None;
        golem.extra.anger = 300;
        golem.extra.angry_at = Some(Who::Player);
        // The attacker (conn 7) ran off; a bystander (conn 8) is nearer.
        let guests = vec![(7, view(30.0, false)), (8, view(3.0, false))];
        assert_eq!(player_for(&golem, None, &guests, Some(7)), Some(7));
        // Calm again (`stopBeingAngry`): the nearest player is seen.
        golem.extra.anger = 0;
        golem.extra.angry_at = None;
        assert_eq!(player_for(&golem, None, &guests, Some(7)), Some(8));
    }
}
