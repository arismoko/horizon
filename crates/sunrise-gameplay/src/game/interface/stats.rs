//! Player statistics (`ServerStatsCounter`): the custom counters, blocks
//! mined, items used/crafted/broken/picked up/dropped and mobs killed or
//! killed by, saved to `players/stats/<uuid>.json`, plus the phantom
//! spawner that reads the time since rest (`PhantomSpawner`).
use super::Dim;
use std::path::PathBuf;
use sunrise_core::{geometry::Vec3, identifier::Identifier, random::RandomSource};
use sunrise_gameplay_common::game::PhantomSpawnResult;

pub use sunrise_gameplay_common::stats::{Stats, locate_player_json, player_json};
use sunrise_gameplay_common::stats::{namespaced, stat_registry};

/// One player `PhantomSpawner.tick` visits. `identity` is the server player's uuid and
/// name, absent for a game's own player, which no server player stands for.
struct PhantomTarget {
    position: Vec3,
    since_rest: i32,
    identity: Option<(u128, String)>,
}

impl crate::game::ServerGame<'_> {
    /// Resolve a hook's complete award before any statistic, scoreboard or packet is changed.
    /// Only listener calls allocate; ordinary per-tick awards keep their borrowed identifiers.
    pub(in crate::game::server_side) fn edited_stat_award(
        &mut self,
        player: u128,
        name: &str,
        ty: &str,
        key: &str,
        n: i32,
    ) -> Option<(String, String, i32)> {
        let mut new_ty = ty.to_owned();
        let mut new_key = key.to_owned();
        let mut amount = n;
        let mut cancel = false;
        self.native_player_stat_award(
            player,
            name,
            &mut new_ty,
            &mut new_key,
            &mut amount,
            &mut cancel,
        );
        if cancel {
            return None;
        }
        let valid = amount >= 0
            && stat_registry(&new_ty).is_some_and(|registry| {
                // Unlike a save document, rewrites must already use the canonical spelling
                // shared by counters, criteria and packets.
                let (Some(ty), Some(key)) = (
                    Identifier::parse_namespaced(&new_ty),
                    Identifier::parse_namespaced(&new_key),
                ) else {
                    return false;
                };
                self.stat_registry_contains("minecraft:stat_type", &ty)
                    && self.stat_registry_contains(registry, &key)
            });
        if valid {
            Some((new_ty, new_key, amount))
        } else {
            static WARNED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!(
                    "player_stat_award refused invalid rewrite ({new_ty}/{new_key}, {amount}); retaining original award"
                );
            }
            Some((ty.to_owned(), key.to_owned(), n))
        }
    }
    pub(in crate::game::server_side) fn stat_registry_contains(
        &self,
        registry: &str,
        id: &Identifier,
    ) -> bool {
        if let Some(host) = self.server.lan.as_ref() {
            return Identifier::parse_namespaced(registry)
                .is_some_and(|registry| host.hub.synced_id(&registry, id).is_some());
        }
        self.common
            .ui
            .registries
            .ids(registry)
            .iter()
            .any(|entry| entry.split_once(':') == Some((id.namespace(), id.path())))
    }

    pub fn stats_path(&self) -> Option<PathBuf> {
        Some(locate_player_json(
            &self.server.world_root,
            "stats",
            self.common.level.player_uuid?,
            &self.common.options.player_name(),
        ))
    }
    /// `ServerStatsCounter.parseLocal`.
    pub(in crate::game) fn load_stats(&mut self) {
        if self.common.stats.loaded || !self.common.is_logical_server() || self.no_player_here() {
            return;
        }
        self.common.stats.loaded = true;
        if let Some(path) = self.stats_path() {
            self.common.stats.values = Stats::read(&path).values;
        }
    }
    /// `ServerStatsCounter.save`.
    pub fn save_stats(&self) {
        if !self.common.stats.loaded
            || self.no_player_here()
            || self
                .server
                .crash_save
                .holds_interrupted(self.server.lan_acting)
        {
            return;
        }
        if let Some(path) = self.stats_path() {
            self.common.stats.write(&path);
        }
    }
    /// `ServerPlayer.awardStat`: `n` more of the statistic `ty` (a bare type
    /// such as `used`) and `key`, unless a native mod cancels the award.
    pub fn award_stat(&mut self, ty: &str, key: &str, n: i32) {
        if self.server.lan_acting_fake {
            return;
        }
        self.award_stat_to(
            ty,
            key,
            n,
            |game| {
                game.server
                    .lan_acting_uuid
                    .zip(game.server.lan_acting_name.clone())
            },
            |game| &mut game.stats,
        );
    }

    pub(in crate::game) fn server_award_stat(&mut self, ty: &str, key: &str, n: i32) {
        self.award_stat(ty, key, n);
    }

    /// Edit the award before borrowing its owner's stats. The normal counter,
    /// dirty set and scoreboard event are all written by `Stats::add`.
    pub(in crate::game::server_side) fn award_stat_to(
        &mut self,
        ty: &str,
        key: &str,
        n: i32,
        identity: impl FnOnce(&Self) -> Option<(u128, String)>,
        stats: impl FnOnce(&mut Self) -> &mut Stats,
    ) {
        // No listener, no player name copied: per-tick counters come through here.
        if mod_api::__private::listened(mod_api::HookId::PlayerStatAward)
            && let Some((player, name)) = identity(self)
        {
            if let Some((ty, key, amount)) = namespaced(ty, |stat_type| {
                self.edited_stat_award(player, &name, stat_type, key, n)
            }) {
                stats(self).add(ty.strip_prefix("minecraft:").unwrap_or(&ty), &key, amount);
            }
        } else {
            stats(self).add(ty, key, n);
        }
    }
    /// `Player.awardStat(Identifier)`: `n` more of the custom statistic `key`.
    pub(in crate::game) fn award_custom_stat(&mut self, key: &str, n: i32) {
        namespaced(key, |key| self.award_stat("custom", key, n));
    }

    pub(in crate::game) fn server_award_custom_stat(&mut self, key: &str, n: i32) {
        self.award_custom_stat(key, n);
    }
    /// `ServerPlayer.awardStat` for a causing entity, regardless of who owns the acting turn.
    pub(in crate::game::server_side) fn award_custom_stat_for(
        &mut self,
        player: u32,
        key: &str,
        n: i32,
    ) {
        use crate::game::server_side::lan_actions_server::CombatPlayer;
        match self.combat_player(player) {
            Some(CombatPlayer::InPlace) => self.award_custom_stat(key, n),
            Some(CombatPlayer::Displaced(depth, _)) => self.award_displaced_stat(depth, key, n),
            Some(CombatPlayer::Parked(guest)) => {
                self.as_guest(guest.conn, |g| g.award_custom_stat(key, n));
            }
            None => {}
        }
    }

    pub(in crate::game::server_side) fn award_displaced_stat(
        &mut self,
        depth: usize,
        key: &str,
        n: i32,
    ) {
        let frame = &self.server.lan_displaced_players.0[depth];
        if frame.fake {
            return;
        }
        namespaced(key, |key| {
            self.award_stat_to(
                "custom",
                key,
                n,
                |game| {
                    let frame = &game.server.lan_displaced_players.0[depth];
                    frame.uuid.map(|uuid| {
                        (
                            uuid,
                            frame
                                .name
                                .clone()
                                .unwrap_or_else(|| super::chat_common::PLAYER_NAME.to_owned()),
                        )
                    })
                },
                |game| &mut game.server.lan_displaced_players.0[depth].stats,
            );
        });
    }
    /// The per-tick counters (`Player.tick`/`ServerPlayer.tick` awardStat).
    pub(in crate::game::server_side) fn tick_player_stat_counters(&mut self) {
        self.load_stats();
        self.count_tick_stats(self.common.sleeping.is_some());
        self.tick_movement_stats();
    }
    /// `Player.tick`'s counters for the player in `self.player`.
    pub(in crate::game::server_side) fn count_tick_stats(&mut self, sleeping: bool) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let alive = !p.health.dead_or_dying();
        let crouching = p.pose == sunrise_sim::pose::Pose::Crouching;
        self.award_custom_stat("play_time", 1);
        self.award_custom_stat("total_world_time", 1);
        if alive {
            self.award_custom_stat("time_since_death", 1);
        }
        if crouching {
            self.award_custom_stat("sneak_time", 1);
        }
        // ServerPlayer.doTick: dead or alive, every tick out of bed.
        if !sleeping {
            self.award_custom_stat("time_since_rest", 1);
        }
    }
    fn tick_movement_stats(&mut self) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let d = p.position.subtract(self.common.previous_position);
        self.movement_stats(d);
    }
    /// `Player.checkMovementStatistics` / `checkRidingStatistics` for one move.
    pub(in crate::game::server_side) fn movement_stats(&mut self, d: Vec3) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let horizontal = ((d.x * d.x + d.z * d.z).sqrt() * 100.0).round() as i32;
        let total = ((d.x * d.x + d.y * d.y + d.z * d.z).sqrt() * 100.0).round() as i32;
        // Teleports are not travel.
        if total <= 0 || total > 1000 {
            return;
        }
        let crouching = p.pose == sunrise_sim::pose::Pose::Crouching;
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        let eye_in_water = self.common.res.facts().get(self.common.world.block(
            feet[0],
            (p.position.y + f64::from(p.dimensions.eye_height)).floor() as i32,
            feet[2],
        ));
        let in_water = self
            .common
            .res
            .facts()
            .get(self.common.world.block(feet[0], feet[1], feet[2]))
            .fluid
            == sunrise_items::blocks::Fluid::Water;
        let on_ground = p.ground.on_ground;
        let (sprinting, gliding, swimming) = (
            p.sprinting,
            p.fall_flying,
            p.pose == sunrise_sim::pose::Pose::Swimming,
        );
        let climbing = p.last_climbable.is_some() && d.y > 0.0;
        let riding = self.common.riding.map(|id| {
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.species())
                .ok_or(())
        });
        match riding {
            Some(Ok(sp)) => {
                use sunrise_sim::mobs::Species;
                // `checkRidingStatistics`: only these mounts count, the horse
                // family by the full 3D distance; any other mount nothing.
                match sp {
                    Species::Pig => self.award_custom_stat("pig_one_cm", total),
                    Species::Strider => self.award_custom_stat("strider_one_cm", total),
                    Species::HappyGhast => self.award_custom_stat("happy_ghast_one_cm", total),
                    Species::Nautilus | Species::ZombieNautilus => {
                        self.award_custom_stat("nautilus_one_cm", total)
                    }
                    sp if sunrise_sim::mobs::equine::is_equine(sp) => {
                        self.award_custom_stat("horse_one_cm", total)
                    }
                    _ => {}
                }
            }
            Some(Err(())) => {
                if self
                    .common
                    .boats
                    .iter()
                    .any(|b| Some(b.id) == self.common.riding)
                {
                    self.award_custom_stat("boat_one_cm", total)
                } else if self
                    .common
                    .carts
                    .iter()
                    .any(|c| Some(c.id) == self.common.riding)
                {
                    self.award_custom_stat("minecart_one_cm", total)
                }
            }
            None if swimming => self.award_custom_stat("swim_one_cm", total),
            None if eye_in_water.fluid == sunrise_items::blocks::Fluid::Water => {
                self.award_custom_stat("walk_under_water_one_cm", total)
            }
            None if in_water => {
                if horizontal > 0 {
                    self.award_custom_stat("walk_on_water_one_cm", horizontal)
                }
            }
            None if climbing => {
                self.award_custom_stat("climb_one_cm", (d.y * 100.0).round() as i32)
            }
            None if on_ground => {
                if horizontal > 0 {
                    if sprinting {
                        self.award_custom_stat("sprint_one_cm", horizontal)
                    } else if crouching {
                        self.award_custom_stat("crouch_one_cm", horizontal)
                    } else {
                        self.award_custom_stat("walk_one_cm", horizontal)
                    }
                }
            }
            None if gliding => self.award_custom_stat("aviate_one_cm", total),
            // Airborne and not gliding: more than 25 cm sideways per move
            // (creative flight, or a long jump), not only flying.
            None if horizontal > 25 => self.award_custom_stat("fly_one_cm", horizontal),
            None => {}
        }
    }
    /// `PhantomSpawner.tick` (`ServerLevel.tickCustomSpawners`): tired players
    /// under an open night sky draw phantoms. `spawn_enemies` is the chunk
    /// source's stored setting, independent of Peaceful difficulty. The game's
    /// own and hosted players count (a server thread's players are all hosted).
    pub(in crate::game) fn tick_phantom_spawner(&mut self, spawn_enemies: bool) {
        if !spawn_enemies
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_PHANTOMS)
            || self.common.dim != Dim::Overworld
        {
            return;
        }
        if self.common.stats.phantom_tick_down() > 0 {
            return;
        }
        let r = &mut self.common.world.random;
        self.common
            .stats
            .phantom_tick_add((60 + r.next_int_bound(60).unwrap_or(0)) * 20);
        if self.sky_darken() < 5 {
            return;
        }
        let rest = |s: &Stats| s.get("custom", "minecraft:time_since_rest");
        // `level.players()`: each player who isn't a spectator.
        let mut players = Vec::new();
        if let Some(p) = self.common.player.as_ref()
            && !self.common.spectator()
        {
            players.push(PhantomTarget {
                position: p.position,
                since_rest: rest(&self.common.stats),
                identity: None,
            });
        }
        if let Some(host) = self.server.lan.as_ref() {
            let level = self.common.dim.id();
            for g in host.hub.guests.iter().filter(|g| {
                g.placed
                    && g.participates_online()
                    && g.dimension == level
                    && g.game_mode != sunrise_sim::lan::guest::SPECTATOR
            }) {
                let since = host.play.get(&g.conn).map_or(0, |p| rest(&p.stats));
                players.push(PhantomTarget {
                    position: Vec3::new(g.position[0], g.position[1], g.position[2]),
                    since_rest: since,
                    identity: (!mod_api::__private::mods().is_empty())
                        .then(|| (g.uuid, g.name.clone())),
                });
            }
        }
        for target in players {
            self.spawn_phantoms_near(target);
        }
    }
    /// Direct test entry for `PhantomSpawner.tick` with the chunk pass's eligibility check.
    #[cfg(any(test, feature = "test-support"))]
    pub fn spawn_phantoms(&mut self) {
        if !self.server.world_debug
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
        {
            self.tick_phantom_spawner(self.server.chunk_spawn_enemies);
        }
    }
    /// One player's turn of `PhantomSpawner.tick`.
    fn spawn_phantoms_near(&mut self, target: PhantomTarget) {
        let PhantomTarget {
            position,
            since_rest,
            identity,
        } = target;
        let pos = [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ];
        let difficulty_id = self.difficulty_id();
        let mut result = None;
        let mut phantoms = None;
        if let Some((player, name)) = identity {
            phantoms = Some(phantom_group_size(
                &mut self.common.world.random,
                difficulty_id,
            ));
            self.native_player_spawn_phantoms(player, &name, pos, &mut result, &mut phantoms);
        }
        let attempt = match result.unwrap_or(PhantomSpawnResult::Default) {
            PhantomSpawnResult::Allow => true,
            PhantomSpawnResult::Default => {
                pos[1] >= self.common.dim.sea_level()
                    && self.mob_world_base(&[]).can_see_sky(pos[0], pos[1], pos[2])
            }
            PhantomSpawnResult::Deny => false,
            _ => unreachable!("PhantomSpawnResult: unhandled variant"),
        };
        if !attempt {
            return;
        }
        // `getCurrentDifficultyAt(playerPos).isHarderThan(random * 3)`.
        let difficulty = self
            .common
            .local_difficulty([position.x, position.y, position.z]);
        let r = &mut self.common.world.random;
        if difficulty.effective <= r.next_float() * 3.0 {
            return;
        }
        if r.next_int_bound(since_rest.max(1)).unwrap_or(0) < 72000 {
            return;
        }
        // `playerPos.above(20 + nextInt(15)).east(-10 + nextInt(21)).south(...)`:
        // the height is drawn first.
        let up = 20 + r.next_int_bound(15).unwrap_or(0);
        let east = -10 + r.next_int_bound(21).unwrap_or(10);
        let south = -10 + r.next_int_bound(21).unwrap_or(10);
        let at = [pos[0] + east, pos[1] + up, pos[2] + south];
        if !self
            .common
            .res
            .facts()
            .is_air(self.common.world.block(at[0], at[1], at[2]))
        {
            return;
        }
        let group = phantoms
            .unwrap_or_else(|| phantom_group_size(&mut self.common.world.random, difficulty_id));
        for _ in 0..group {
            let _ = self.spawn_mob(
                "minecraft:phantom",
                [
                    f64::from(at[0]) + 0.5,
                    f64::from(at[1]),
                    f64::from(at[2]) + 0.5,
                ],
                false,
                sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                    sunrise_sim::mobs::spawn_rules::Reason::Natural,
                ),
            );
        }
    }
}

/// `1 + random.nextInt(difficulty.getId() + 1)` of `PhantomSpawner.tick`: Peaceful still draws.
fn phantom_group_size(random: &mut impl RandomSource, difficulty_id: i32) -> i32 {
    1 + random.next_int_bound(difficulty_id + 1).unwrap_or(0)
}

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phantom_group_size_follows_difficulty() {
        // Expected from java.util.Random(12345): 1 + nextInt(id + 1), six draws each.
        let expected: [(i32, [i32; 6]); 4] = [
            (0, [1, 1, 1, 1, 1, 1]),
            (1, [1, 2, 2, 2, 2, 1]),
            (2, [2, 2, 1, 1, 2, 2]),
            (3, [2, 3, 4, 4, 4, 1]),
        ];
        for (id, sizes) in expected {
            let mut r = sunrise_core::random::LegacyRandom::new(12345);
            let got: Vec<i32> = (0..6).map(|_| phantom_group_size(&mut r, id)).collect();
            assert_eq!(got, sizes, "difficulty {id}");
        }
    }
}
