//! The locator bar of the players on a served world (lan/waypoints.rs): what
//! each player looks like to their level's waypoint manager this tick.
use super::commands::Target;
use sunrise_sim::lan::waypoints::{self, PLAYER_RANGE};
use sunrise_sim::motion::MotionEvent;

impl crate::game::ServerGame<'_> {
    /// `ServerLevel.EntityCallbacks.onTrackingStart`: register from the loaded
    /// attributes. Saved equipment contributes modifiers on the first living tick.
    pub fn lan_register_player_waypoint(&mut self, conn: u32) {
        self.as_guest(conn, |game| {
            game.update_waypoint_transmitter(Target::Guest(conn));
        });
    }

    /// `LivingEntity.onAttributeUpdated` runs both at a player tick and on
    /// effect updates/removals. Dispatch its transmit requests where it runs.
    pub(in crate::game) fn apply_player_waypoint_updates(&mut self) {
        let Some(player) = self.common.player.as_mut() else {
            return;
        };
        let updates: Vec<_> = player
            .events
            .extract_if(.., |event| {
                matches!(
                    event,
                    MotionEvent::Waypoint {
                        receiver: false,
                        ..
                    }
                )
            })
            .collect();
        let target = self.waypoint_target(Target::Player);
        for event in updates {
            if let MotionEvent::Waypoint { enabled, .. } = event {
                self.set_waypoint_transmitter(target, enabled);
            }
        }
    }

    fn waypoint_player(&self, g: &sunrise_sim::lan::guest::Guest) -> waypoints::Player {
        let body = if self.server.lan_acting == Some(g.conn) {
            self.common.player.as_ref()
        } else {
            g.motion.as_deref()
        };
        waypoints::Player {
            uuid: g.uuid,
            conn: Some(g.conn),
            position: body.map_or(g.position, |p| p.position.components()),
            spectator: g.spectator(),
            transmit_range: body.map_or(PLAYER_RANGE, |p| {
                p.attributes
                    .computed_value_of("minecraft:waypoint_transmit_range")
                    .unwrap_or_default()
            }),
            transmitting: self
                .server
                .cmds
                .0
                .waypoints
                .as_ref()
                .is_some_and(|tracked| tracked.contains(&Target::Guest(g.conn))),
            receive_range: PLAYER_RANGE,
            first_tick: g.ticks == 0,
            view_distance: g.view_distance,
            team: self.common.score.board.players_team(&g.name).map(|team| {
                (
                    team.name.clone(),
                    team.color.as_deref().and_then(waypoints::team_rgb),
                )
            }),
            icon: self
                .server
                .cmds
                .0
                .waypoint_icons
                .get(&Target::Guest(g.conn))
                .cloned()
                .unwrap_or_default(),
        }
    }

    pub(in crate::game::server_side) fn lan_set_waypoint_transmitting(
        &mut self,
        conn: u32,
        enabled: bool,
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        let level = guest.dimension.clone();
        if !host.waypoints.contains_key(&level) {
            return;
        }
        let uuid = guest.uuid;
        let players: Vec<_> = host
            .hub
            .online_players()
            .filter(|g| g.dimension == level)
            .map(|g| self.waypoint_player(g))
            .collect();
        let locator_bar = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::LOCATOR_BAR);
        if let Some(host) = self.server.lan.as_mut()
            && let Some(manager) = host.waypoints.get_mut(&level)
        {
            for (conn, packet) in manager.set_transmitting(&players, uuid, enabled, locator_bar) {
                if let Some(g) = host.hub.guest(conn) {
                    g.send(&packet);
                }
            }
        }
    }

    /// Each level's `ServerWaypointManager` ticked with the players in that
    /// level: a player who changes level leaves the old one's manager
    /// (`ServerLevel.removePlayerImmediately`: untracked for everyone there)
    /// and joins the new one's (`addDuringTeleport`).
    pub fn lan_update_waypoints(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let players: Vec<_> = host
            .hub
            .online_players()
            .map(|g| (g.dimension.clone(), g.dead, self.waypoint_player(g)))
            .collect();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let mut respawned = Vec::new();
        for (_, dead, p) in &players {
            // PlayerList.respawn makes a new ServerPlayer.
            if *dead {
                host.waypoint_dead.insert(p.uuid);
            } else if host.waypoint_dead.remove(&p.uuid) {
                respawned.push(p.uuid);
            }
        }
        host.waypoint_dead
            .retain(|uuid| players.iter().any(|(_, _, p)| p.uuid == *uuid));
        let locator_bar = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::LOCATOR_BAR);
        let mut levels: Vec<String> = host.waypoints.keys().cloned().collect();
        for (level, _, _) in &players {
            if !levels.iter().any(|l| l == level) {
                levels.push(level.clone());
            }
        }
        levels.sort();
        let mut out = Vec::new();
        for level in levels {
            let here: Vec<waypoints::Player> = players
                .iter()
                .filter(|(l, _, _)| *l == level)
                .map(|(_, _, p)| p.clone())
                .collect();
            let away: Vec<(u128, u32)> = players
                .iter()
                .filter(|(l, _, _)| *l != level)
                .filter_map(|(_, _, p)| Some((p.uuid, p.conn?)))
                .collect();
            let manager = host.waypoints.entry(level.clone()).or_default();
            out.extend(manager.tick_level(&here, &away, &respawned, locator_bar));
            // Nobody left in the level: nothing to remember there.
            if here.is_empty() {
                host.waypoints.remove(&level);
            }
        }
        for (conn, packet) in out {
            if let Some(g) = host.hub.guest(conn) {
                g.send(&packet);
            }
        }
    }
}
