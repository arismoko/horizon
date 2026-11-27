//! What `sunrise-server` asks the dedicated server on its control endpoint (`--control-socket`, docs/server.md
//! section 2): `server-status` answers with the tick rate, the tick times `/tick query` reports, the players
//! (including their positions) and the loaded chunks; `server-mods` answers with the canonical loaded package
//! inventory; `console <line>` runs a console command and answers with the lines it logged;
//! `server-events <since>` answers the typed events after that number (events.rs); `management <connection>
//! <json>` edits the lists as vanilla's management protocol does (management.rs).
//! `player-expect <uuid> [seconds]`, `player-drop <uuid>`, `player-export <uuid> [freeze]`,
//! `player-release <uuid> ...`, `player-import <uuid> <json>` and `player-confirm <uuid> ...` are a proxy's,
//! for a player it moves between two servers, and `transfer-status` the counts it reads first (transfer.rs).
use super::{ServerState, capture_log};
use crate::game::ServerGame;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

/// The window the tick rate is measured over.
const TPS_WINDOW: Duration = Duration::from_secs(5);

fn record_tick(ticks: &mut VecDeque<Instant>, at: Instant) {
    ticks.push_back(at);
    // Keep the boundary before the window, including the previous completion
    // when a single tick takes longer than the window. Otherwise every slow
    // completion prunes its predecessor and the measured rate is always zero.
    while ticks.len() > 2 && at.duration_since(ticks[1]) > TPS_WINDOW {
        ticks.pop_front();
    }
}

fn measured_tps(ticks: &VecDeque<Instant>, include_boundary: bool) -> f64 {
    let (Some(first), Some(last)) = (ticks.front(), ticks.back()) else {
        return 0.0;
    };
    let boundary = usize::from(!include_boundary && last.duration_since(*first) > TPS_WINDOW);
    let intervals = ticks.len() - 1 - boundary;
    let elapsed = last.duration_since(ticks[boundary]).as_secs_f64().max(1e-9);
    intervals as f64 / elapsed
}

impl ServerState {
    /// One pass of the server loop ended.
    pub(super) fn ticked(&mut self, at: Instant) {
        record_tick(&mut self.ticks, at);
    }
    /// Ticks per second over the last few seconds.
    pub(super) fn tps(&self) -> f64 {
        measured_tps(&self.ticks, true)
    }

    /// Keep shutdown hand-over timing on the original five-second window.
    pub(super) fn transfer_tps(&self) -> f64 {
        measured_tps(&self.ticks, false)
    }
}

impl ServerGame<'_> {
    /// Answers the requests waiting on the control endpoint.
    pub(super) fn poll_server_control(&mut self, state: &mut ServerState) {
        while let Some(request) = self.common.control.as_ref().and_then(|c| c.try_recv()) {
            // A `console stop` just before stops this server for the next request too.
            self.note_stopping(state.lists.lock().unwrap().stop);
            let reply = match request.line.trim().split_once(' ') {
                _ if request.line.trim() == "server-status" => self.server_status(state),
                _ if request.line.trim() == "server-mods" => server_mods(),
                // A proxy reading whether it may send players here (transfer.rs).
                _ if request.line.trim() == "transfer-status" => self.transfer_status(),
                Some(("server-events", since)) => {
                    super::events::since(since.trim().parse().unwrap_or(0))
                }
                Some(("management", rest)) => self.management(state, rest),
                // A proxy moving a player here or away (transfer.rs, lan/arrivals.rs).
                Some((verb, rest)) if verb.starts_with("player-") => {
                    self.player_request(verb, rest)
                }
                Some(("console", line)) => {
                    // Command feedback is logged as system chat; the reply carries the text alone.
                    let output: Vec<String> = capture_log(|| self.console_command(state, line))
                        .into_iter()
                        .map(|l| {
                            l.strip_prefix("System chat: ")
                                .map_or(l.clone(), str::to_owned)
                        })
                        .collect();
                    json!({ "output": output })
                }
                // An RCON command: run as the console, with the `Rcon` source, whose feedback the
                // operators are told of when `broadcast-rcon-to-ops` is on.
                Some(("rcon", line)) => {
                    let output: Vec<String> = capture_log(|| self.rcon_line(state, line))
                        .into_iter()
                        .map(|l| {
                            l.strip_prefix("System chat: ")
                                .map_or(l.clone(), str::to_owned)
                        })
                        .collect();
                    json!({ "output": output })
                }
                _ => json!({ "error": format!("unknown request {:?}", request.line) }),
            };
            let _ = request.reply.send(reply.to_string());
        }
    }

    /// Each level's loaded chunks and entities (players and block entities apart), the level in place
    /// first: what the Prometheus text reports by dimension.
    fn status_levels(&self) -> Value {
        let mut levels = vec![json!({
            "dimension": self.dim.id(),
            "chunks": self.world.chunks.len(),
            "entities": self.entity_count(),
        })];
        for dim in self.server.levels.0.parked_dims() {
            if let (Some(world), Some(entities)) = (
                self.server.levels.0.parked_world(dim),
                self.server.levels.0.parked_entity_count(dim),
            ) {
                levels.push(json!({
                    "dimension": dim.id(),
                    "chunks": world.chunks.len(),
                    "entities": entities,
                }));
            }
        }
        Value::Array(levels)
    }

    /// The entities of the level in place: mobs, items, orbs, projectiles, vehicles, falling blocks,
    /// hangings, armor stands, cushions, tridents, rockets and primed TNT.
    fn entity_count(&self) -> usize {
        self.common.mobs.iter().filter(|m| !m.removed).count()
            + self.common.dropped.len()
            + self.common.orbs.len()
            + self.common.creatures.projectiles.len()
            + self.common.carts.len()
            + self.common.boats.len()
            + self.common.falling.len()
            + self.common.paintings.len()
            + self.common.item_frames.len()
            + self.common.armor_stands.len()
            + self.common.cushions.len()
            + self.common.tridents.len()
            + self.common.rockets.len()
            + self.common.tnts.len()
    }

    fn server_status(&self, state: &ServerState) -> Value {
        let millis = |nanos: u64| nanos as f64 / 1e6;
        let samples = self.common.clock.sorted_samples();
        // The percentiles `/tick query` prints (TickCommand.percentileToString).
        let at = |q: f64| {
            let i = ((samples.len() as f64 * q) as usize).min(samples.len().saturating_sub(1));
            samples.get(i).copied().map_or(0.0, millis)
        };
        let (players, max_players) = self.server.lan.as_ref().map_or((Vec::new(), 0), |h| {
            let players = h
                .hub
                .guests
                .iter()
                .filter(|g| g.participates_online())
                .map(|g| {
                    let mut status = player_status(
                        &g.name,
                        g.uuid,
                        g.ip_address(),
                        g.latency.load(Ordering::Relaxed),
                        &g.dimension,
                        g.position,
                    );
                    // Charged bytes held for transmission, including packet bookkeeping.
                    status["pending_send_bytes"] = json!(g.sender.pending.load(Ordering::Relaxed));
                    status
                })
                .collect();
            (players, h.hub.lan.shared.max_players())
        });
        json!({
            "tps": state.tps(),
            "target_tps": self.clock.rate(),
            "mspt": {
                "average": millis(self.clock.average_nanos()),
                "p50": at(0.5),
                "p95": at(0.95),
                "p99": at(0.99),
            },
            "players": players,
            "max_players": max_players,
            "chunks": self.world.chunks.len(),
            "levels": self.status_levels(),
            "paused": state.paused,
        })
    }
}

/// Reads the same package inventory as registration and the runtime Host.
fn server_mods() -> Value {
    let mods: Vec<_> = mod_api::__private::loaded_mods()
        .iter()
        .map(|package| json!({ "id": package.id(), "version": package.version() }))
        .collect();
    json!({ "mods": mods })
}

fn player_status(
    name: &str,
    uuid: u128,
    address: &str,
    ping_ms: i32,
    dimension: &str,
    position: [f64; 3],
) -> Value {
    json!({
        "name": name,
        "uuid": sunrise_protocol::crypto::uuid_string(uuid),
        "address": address,
        "ping_ms": ping_ms,
        "dimension": dimension,
        "position": position,
    })
}

#[cfg(test)]
mod tests {
    use super::{player_status, server_mods};
    #[test]
    fn slow_ticks_keep_a_measurable_interval_and_recover() {
        let mut ticks = std::collections::VecDeque::new();
        let start = std::time::Instant::now();
        super::record_tick(&mut ticks, start);
        super::record_tick(&mut ticks, start + std::time::Duration::from_secs(20));
        assert_eq!(super::measured_tps(&ticks, true), 0.05);
        assert_eq!(super::measured_tps(&ticks, false), 0.0);
        super::record_tick(&mut ticks, start + std::time::Duration::from_millis(20_050));
        assert_eq!(super::measured_tps(&ticks, false), 20.0);
        for tick in 2..=120 {
            super::record_tick(
                &mut ticks,
                start + std::time::Duration::from_millis(20_000 + tick * 50),
            );
        }
        assert_eq!(super::measured_tps(&ticks, true), 20.0);
    }
    use serde_json::json;

    #[test]
    fn server_mods_uses_canonical_inventory_order() {
        mod_platform::__private::with_test_packages(&["second", "first"], || {
            assert_eq!(
                server_mods(),
                json!({"mods": [
                    {"id": "second", "version": "1"},
                    {"id": "first", "version": "1"},
                ]})
            );
        });
        mod_platform::__private::with_test_packages(&[], || {
            assert_eq!(server_mods(), json!({"mods": []}));
        });
    }

    #[test]
    fn player_status_includes_coordinates() {
        assert_eq!(
            player_status(
                "Alex",
                1,
                "203.0.113.7",
                42,
                "minecraft:the_nether",
                [1.5, 64.0, -2.25]
            ),
            json!({
                "name": "Alex",
                "uuid": "00000000-0000-0000-0000-000000000001",
                "address": "203.0.113.7",
                "ping_ms": 42,
                "dimension": "minecraft:the_nether",
                "position": [1.5, 64.0, -2.25],
            })
        );
    }
}
