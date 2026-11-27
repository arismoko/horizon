//! The view pass (#3716): a level that only viewed `Loaded` areas name is not ticked, but its
//! chunks are loaded, generated and sent to the players whose clients view it.
//!
//! After the ticked levels, `tick_server` runs a pass for each such level: it enters the level,
//! rebuilds `Streaming.extra` (`lan_update_streaming`, where the areas are added: no `tick_lan`
//! runs here to call it), collects the finished columns (`update_world`), postprocesses them as
//! vanilla does before a chunk is sent (`postprocess_chunks`: what it schedules waits for a tick),
//! then runs only the hub's viewed-level sending, with the level's entities as they stand for its
//! viewers to track. Nothing ticks there: no block, fluid, scheduled or random ticks, no
//! entities, no weather cycle, no level tick hooks and no player work. Its time is the server's,
//! as in 26.3, where no level but the Overworld runs a clock of its own: `enter_level` gives it
//! the clocks as they are, and its viewers get its game time every 20 ticks. A level whose last
//! area was withdrawn keeps its passes, with no ticket, until its chunks were forgotten and it was
//! closed on the clients.
use super::Dim;
use sunrise_game_data::Result;
use sunrise_sim::lan::hub::TickCtx;

impl crate::game::ServerGame<'_> {
    /// What the players' viewed areas ask of level `dim`: None for nothing, else whether one of
    /// them ticks.
    pub fn viewed_level(&self, dim: Dim) -> Option<bool> {
        self.server.lan.as_ref()?.hub.viewed_level(dim.id())
    }

    /// Whether level `dim` needs a view pass when it is not ticked: areas name it, or a client
    /// still holds chunks of it whose linger, forgets and `close` are not done.
    pub fn needs_view_pass(&self, dim: Dim) -> bool {
        self.viewed_level(dim).is_some()
            || self
                .server
                .lan
                .as_ref()
                .is_some_and(|h| h.hub.viewed_open(dim.id()))
    }

    /// A chunk pass for each idle level held by a view or a spawn preparation ticket.
    pub(in crate::game::server_side) fn run_view_passes(
        &mut self,
        ticked: &[Dim],
        running: bool,
    ) -> Result<()> {
        let viewed: Vec<Dim> = Dim::all()
            .into_iter()
            .filter(|d| {
                !ticked.contains(d)
                    && (self.needs_view_pass(*d)
                        || self.has_spawn_tickets(*d)
                        || self.has_async_chunk_requests(*d))
            })
            .collect();
        for dim in viewed {
            let _span = sunrise_level::spans::guard("view_pass");
            self.enter_level(dim)?;
            self.update_dedicated_streaming();
            self.lan_update_streaming();
            // This is not a level tick: rebuilding its loading levels tells no listeners.
            self.tick_spawn_tickets(running, false);
            self.server_update_world();
            // `LevelChunk.postProcessGeneration` comes before a chunk is sent
            // (`ChunkMap.prepareTickingChunk`, then `onChunkReadyToSend`), ticked or not.
            self.postprocess_chunks()?;
            // Like LevelChunk.registerAllBlockEntitiesAfterLevelLoad, attaching a loaded
            // entity publishes it even when this level has no entity ticks.
            self.fire_pending_block_entity_loads();
            if self.needs_view_pass(dim) {
                self.view_pass_send();
            }
        }
        Ok(())
    }

    /// The hub's viewed-level sending for the level in place, without its tick.
    fn view_pass_send(&mut self) {
        let states = self.view_pass_states();
        let spawn = self.world_spawn_position();
        let clock_rate = self.network_clock_rate();
        let sea_level = self
            .common
            .end
            .locator()
            .map_or(63, |terrain| terrain.sea_level());
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let mut ctx = TickCtx {
            sea_level,
            world: &mut self.common.world,
            entities: states.as_slice(),
            next_entity: &mut self.server.next_entity,
            spawn,
            spawn_angle: self.common.level.world_spawn_angle,
            spawn_dimension: sunrise_gameplay_common::game::Dim::of_world_spawn(&self.common.level)
                .id(),
            difficulty: i32::from(self.common.ui.difficulty),
            difficulty_locked: self.common.level.difficulty_locked,
            clock_rate,
            tick_rate: self.common.clock.rate(),
            tick_frozen: self.common.clock.frozen(),
            tick_steps: self.common.clock.step_state(),
            collision: Some(self.common.collision),
            border: &self.common.border,
            streaming: Some(&self.server.streaming),
        };
        host.hub.viewed_pass(&mut ctx);
    }

    /// The level's entities as `tick_lan` describes them for the tracking pass: nobody is in the
    /// level, so no player rides one.
    fn view_pass_states(&mut self) -> sunrise_sim::lan::entity::States {
        let mut states = sunrise_sim::lan::entity::States::default();
        if !self
            .server
            .lan
            .as_ref()
            .is_some_and(|h| h.hub.viewed_here())
        {
            return states;
        }
        self.lan_assign_ids();
        let Some(host) = self.server.lan.as_mut() else {
            return states;
        };
        let mut riding = std::mem::take(&mut host.hub.riding);
        self.lan_riding(&[], &mut riding);
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.riding = riding;
        }
        self.lan_entities(&mut states, true, 0, None);
        self.lan_hook_states(&mut states);
        self.lan_entity_uuids(&mut states);
        self.lan_syncs_looked();
        states
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use sunrise_gameplay_common::game::server_side::view_pass::tests::sent;

    use super::*;
    use sunrise_protocol::packets::play::Clientbound as C;

    /// Opens the world to the network with one player in the Overworld whose client, of revision
    /// 3, views 2 levels; its packets.
    pub fn host_a_viewer(
        game: &mut crate::game::ServerGame<'_>,
    ) -> std::sync::mpsc::Receiver<sunrise_sim::lan::Out> {
        host_with_viewer(game, || {
            let (tx, rx) = std::sync::mpsc::channel();
            (
                sunrise_sim::lan::PlayerSender::new(tx, std::sync::Arc::default()),
                rx,
            )
        })
    }

    pub fn host_with_viewer<T>(
        game: &mut crate::game::ServerGame<'_>,
        make_sender: impl FnOnce() -> (sunrise_sim::lan::PlayerSender, T),
    ) -> T {
        use sunrise_sim::lan::{Joined, NetConfig, guest::Guest, hub::HubConfig};
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "view-pass-test".into(), 2)
        };
        let settings = crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        };
        let hub = HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        game.lan_host(settings, net, hub).unwrap();
        let (mut sender, receive) = make_sender();
        // Revision 3, still served, names a viewed level by its Login index: the Overworld 0, the
        // Nether 2.
        sender.viewed_levels = sunrise_protocol::viewed_levels::Answer {
            revision: sunrise_protocol::viewed_levels::REVISION_3,
            max_levels: 2,
        };
        let joined = Joined {
            conn: 2,
            uuid: 2,
            name: "Viewer".into(),
            properties: Vec::new(),
            info: sunrise_protocol::packets::configuration::ClientInformation::new(2),
            brand: "sunrise".into(),
            sender,
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        };
        game.server.next_entity += 1;
        let mut guest = Guest::new(joined, game.server.next_entity as i32, [0.5, 80.0, 0.5], 0);
        guest.placed = true;
        guest.loaded = true;
        guest.dimension = Dim::Overworld.id().into();
        guest.max_unacknowledged = 1000;
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        // Its client was told at Login the levels the server has now.
        guest.viewed.logged_in_with(hub.level_keys());
        hub.guests.push(guest);
        receive
    }

    /// Summons `entity` in the Nether, at chunk (40, 40) above its bedrock roof, and gives its id.
    pub fn summon_in_the_nether(game: &mut crate::game::ServerGame<'_>, entity: &str) -> i32 {
        game.enter_level(Dim::Nether).unwrap();
        game.set_allow_commands(true);
        assert!(game.run_command(&format!("summon {entity} 648.5 130 648.5")));
        let summoned = game.mobs.iter().find(|m| m.kind.id == entity).unwrap();
        summoned.id as i32
    }

    /// The viewer, a server player.
    pub fn viewer<'h>(game: &'h crate::game::ServerGame<'_>) -> &'h sunrise_sim::lan::guest::Guest {
        let hub = &game.server.lan.as_ref().unwrap().hub;
        hub.guests.iter().find(|g| g.uuid == 2).unwrap()
    }

    /// Sends the viewer (connection 2) into level `dim` at `at`, as `/tp` into another level
    /// does (`ServerPlayer.teleport(TeleportTransition)`).
    pub fn travel(game: &mut crate::game::ServerGame<'_>, dim: Dim, at: [f64; 3]) {
        game.server
            .level_teleports
            .push(crate::game::server_travel::LevelTeleport {
                conn: 2,
                dim,
                at,
                rotation: None,
                vehicle: None,
            });
        game.run_level_teleports();
    }

    /// Runs `command` in the Overworld, where the player is, then a tick: what the viewer's Nether
    /// was told of its clocks meanwhile, each update's clocks as their totals and rates.
    pub fn clocks_after(
        game: &mut crate::game::ServerGame<'_>,
        rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
        command: &str,
    ) -> Vec<Vec<(i64, f32)>> {
        game.enter_level(Dim::Overworld).unwrap();
        assert!(game.run_command(command), "{command}");
        game.tick_server(false).unwrap();
        (sent(rx).others.into_iter())
            .filter_map(|p| match p {
                C::SetTime(t) => Some(t.clocks.iter().map(|c| (c.total_ticks, c.rate)).collect()),
                _ => None,
            })
            .collect()
    }
}

use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1 as _;
