//! `Game` as the `CustomSpawnerLevel` the custom spawners run on: each service forwards to the game's own method.
use crate::game::ServerGame;
use std::collections::HashSet;
use sunrise_game_data::gamerules::RuleIndex;
use sunrise_gameplay::custom_spawner::{CustomSpawnerLevel, SpawnerParts, SpawnerPlayer};
use sunrise_gameplay_common::game::creatures::SpawnSite;
use sunrise_sim::mobs::{Species, raider, spawn_rules};

impl CustomSpawnerLevel for ServerGame<'_> {
    fn parts(&mut self) -> SpawnerParts<'_> {
        SpawnerParts {
            world: &mut self.common.world,
            mobs: &mut self.common.mobs,
            facts: self.common.res.facts(),
        }
    }
    fn overworld(&self) -> bool {
        self.common.dim == super::Dim::Overworld
    }
    fn rule(&self, rule: RuleIndex) -> bool {
        self.common.ui.rules.bool_at(rule)
    }
    fn players(&self) -> Vec<SpawnerPlayer> {
        self.level_players_iter()
            .map(|p| SpawnerPlayer {
                uuid: p.uuid,
                position: p.position,
                alive: p.alive,
                spectator: p.spectator,
            })
            .collect()
    }
    fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
        crate::game::ServerGame::village_centers(self)
    }
    fn is_spawn_position_ok(&self, kind: &str, species: Species, pos: [i32; 3]) -> bool {
        spawn_rules::is_spawn_position_ok(
            kind,
            species,
            &self.mob_world_base(&[]),
            pos[0],
            pos[1],
            pos[2],
        )
    }
    fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        reason: spawn_rules::Reason,
        yaw: Option<f32>,
    ) -> Option<u32> {
        let site = SpawnSite::Finalizing(reason);
        match yaw {
            Some(y) => self.spawn_mob_facing(kind, at, persistent, site, y),
            None => crate::game::ServerGame::spawn_mob(self, kind, at, persistent, site),
        }
    }
    fn spawn_event_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        leader: bool,
        reason: raider::Reason,
    ) -> Option<u32> {
        crate::game::ServerGame::spawn_event_mob(self, kind, at, leader, reason)
    }
    fn structure_tag(&self, tag: &str) -> Vec<String> {
        crate::game::ServerGame::structure_tag(self, tag)
    }
    fn structures_at(&self, pos: [i32; 3]) -> Vec<String> {
        sunrise_gameplay_common::game::CommonGame::structures_at(self, pos)
    }
}
