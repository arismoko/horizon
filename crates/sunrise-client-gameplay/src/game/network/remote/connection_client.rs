//! The window's connection to a server, as the engine sees it: its record of the last server, the
//! hooks it calls as a connection starts, lets the player in and closes (`sunrise_hooks::connection`),
//! and the Disconnected screen's offer to connect again, which a crate above asks for
//! (`Hooks::retry`). With nothing asking, the screen is vanilla's `DisconnectedScreen`.
//!
//! A connection over the network is one session, from `connection_starts` until it closes, once,
//! in `connection_left` or `connection_ended`, whichever way: the player leaving, the game closing,
//! a transfer, the server, the connection or this client ending it. A world the window plays
//! itself never opens one.
use super::*;
use sunrise_client_lib::{multiplayer, title::Mode};
use sunrise_hooks::connection::{By, Ended, Left, Retry, Server};

/// The Disconnected screen's line about connecting again, while that screen shows.
pub(in crate::game) fn retry_slot(
    title: &mut Option<sunrise_client_lib::title::Title>,
) -> Option<&mut Option<String>> {
    match &mut title.as_mut()?.mode {
        Mode::Multiplayer(multiplayer::Screen::Disconnected { retry, .. }) => Some(retry),
        _ => None,
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A stack to the server (`SetCreativeModeSlot`): the same component
    /// patch a Sunrise server sends its clients, with the registries' ids, so
    /// the server's echo doesn't hand back a bare item.
    pub fn to_network(&self, s: &Option<ItemStack>) -> Option<Stack> {
        self.to_network_projecting(s, &|_| ())
    }

    /// `ConnectScreen`'s Cancel: a server's (single player's server isn't
    /// joined through `ConnectScreen`), until the login is done.
    pub fn connect_cancel(&self, (w, h): (f32, f32)) -> Option<(f32, f32)> {
        let r = self.client.remote.as_ref()?;
        (self.common.player.is_none() && !r.joined && self.client.integrated.is_none())
            .then(|| sunrise_client::level_loading::connect_cancel(w, h))
    }
    /// A click on `ConnectScreen`: Cancel aborts the connection
    /// (`connect.aborted`) and goes back. Escape does nothing there
    /// (`shouldCloseOnEsc` is false).
    pub fn connect_screen_click(&mut self) -> Result<bool> {
        // ServerReconfigScreen's Disconnect, once it is active.
        if let Some(since) = self.client.remote.as_ref().and_then(|r| r.reconfiguring) {
            let (_, (x, y)) = self.reconfigure_layout(self.gui_size());
            if self.common.ticks.saturating_sub(since) >= 600
                && sunrise_client::ui::hovered(self.client.cursor, x, y, 150.0, 20.0)
            {
                self.play_audio_button_click();
                self.leave_server(None)?;
                return Ok(true);
            }
            return Ok(false);
        }
        let Some((x, y)) = self.connect_cancel(self.gui_size()) else {
            return Ok(false);
        };
        if sunrise_client::ui::hovered(self.client.cursor, x, y, 200.0, 20.0) {
            self.play_audio_button_click();
            // ConnectScreen Cancel aborts the pending connection and returns to
            // its parent screen without calling Minecraft.disconnect.
            self.finish_leave_server(None)?;
            return Ok(true);
        }
        Ok(false)
    }
    /// Leave the title for a server (ConnectScreen): an empty world fed by the connection.
    pub(in crate::game) fn connect_server(&mut self, address: &str, name: &str) -> Result<()> {
        self.connect_server_with_options(address, name, None, false, false, None)
    }
    /// `ConnectScreen.startConnecting`: keep LAN discovery's server kind through loading.
    pub fn connect_server_with_options(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<sunrise_client_lib::net::Cookies>,
        lan: bool,
        realm: bool,
        list_name: Option<&str>,
    ) -> Result<()> {
        sunrise_gameplay_common::game::world_preload::world_data_used();
        if self.defer_world_entry(crate::game::PendingWorldEntry::Server {
            address: address.to_owned(),
            name: name.to_owned(),
            list_name: list_name.map(str::to_owned),
            cookies: cookies.clone(),
            lan,
            realm,
        })? {
            return Ok(());
        }
        self.common.join_world_rest()?;
        self.client_logging_out();
        if let Err(error) = self.join_server(address, name, None, cookies, lan, realm, list_name) {
            log::warn!("Sunrise: could not join server: {error}");
            let skip = self.client.settings.file.bool("skipMultiplayerWarning");
            match &mut self.client.title.0 {
                Some(title) => {
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                }
                None => {
                    let mut title = self
                        .new_title(std::env::current_dir()?.join("saves"))
                        .with_user(|| self.common.options.player_name());
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                    self.show_title(title);
                }
            }
            self.capture(false);
        }
        Ok(())
    }
    /// The open network connection ended without the player leaving (`Hooks::ended`), before
    /// the Disconnected screen shows; nothing when none is open. The crates are asked once
    /// whether to connect again (`Hooks::retry`), and that screen acts on the answer
    /// (`offer_retry`); the last server is forgotten unless something will connect there again.
    pub(in crate::game::client_side) fn connection_ended(
        &mut self,
        reason: &str,
        key: Option<&str>,
        by: By,
    ) {
        if !std::mem::take(&mut self.client.connection_open) {
            return;
        }
        sunrise_hooks::engine::ended(&Ended::new(reason, key, by));
        self.client.pending_retry = sunrise_hooks::engine::retry();
        if self.client.pending_retry.is_none() {
            self.client.last_connection = None;
        }
    }
    /// The server let the player in (`Hooks::joined`); nothing for a world the window plays
    /// itself, whose session joins the same way.
    pub(in crate::game::client_side) fn connection_joined(&mut self) {
        if !self.client.connection_open {
            return;
        }
        sunrise_hooks::engine::joined();
    }
    /// The open network connection closes without being lost (`Hooks::left`). Nothing when
    /// none is open: it closed already, or the window plays its own world.
    pub(in crate::game) fn connection_left(&mut self, how: Left) {
        if !std::mem::take(&mut self.client.connection_open) {
            return;
        }
        self.client.last_connection = None;
        sunrise_hooks::engine::left(how);
    }
    /// A connection to `address` as `name` starts over the network (`Hooks::connecting`). One
    /// still open closes first, as the player leaving it.
    pub fn connection_starts(&mut self, address: &str, name: &str, lan: bool) {
        self.connection_left(Left::Disconnected);
        self.client.pending_retry = None;
        self.client.retry_due = None;
        let server = Server::new(address, name, lan);
        sunrise_hooks::engine::connecting(&server);
        self.client.last_connection = Some(server);
        self.client.connection_open = true;
    }
    /// Once a frame while the Disconnected screen offers to connect again: the connection due
    /// from the last answer, else the crates' answer now (its line, the connection, or nothing
    /// any more). Cancelled or left, the offer and its due connection go. An online-mode server
    /// gets the signed-in account again: `connect_server` takes it from `auth::current()`, held
    /// in memory for the whole run (the launcher's session file is gone after the first read).
    pub(in crate::game::client_side) fn follow_retry(&mut self) -> Result<()> {
        if retry_slot(&mut self.client.title.0).is_none_or(|slot| slot.is_none()) {
            self.client.retry_due = None;
            return Ok(());
        }
        let answer = match self.client.retry_due.take() {
            Some(server) => Some(Retry::Now(server)),
            None => sunrise_hooks::engine::retry(),
        };
        let Some(slot) = retry_slot(&mut self.client.title.0) else {
            return Ok(());
        };
        let server = match answer {
            Some(Retry::Wait(line)) => {
                *slot = Some(line);
                return Ok(());
            }
            Some(Retry::Now(server)) => server,
            // Nothing asks any more.
            _ => {
                *slot = None;
                return Ok(());
            }
        };
        *slot = None;
        let realm =
            self.client.realm_session && self.server_address() == Some(server.address.as_str());
        self.connect_server_with_options(
            &server.address,
            &server.name,
            None,
            server.lan,
            realm,
            None,
        )
    }
    // handle_command_suggestions: remote/commands.rs
    /// Another player of this level, whose copy passes every click (`Player.interactOn`).
    pub(in crate::game::client_side) fn is_level_player(&self, id: i32) -> bool {
        self.client
            .remote
            .as_ref()
            .is_some_and(|r| r.level_players.contains_key(&id))
    }
    /// The Disconnected screen just shown offers to connect again when the answer to the
    /// connection's end asked to: its line, or the connection on the next frame (`follow_retry`)
    /// when it is due at once.
    pub(in crate::game::client_side) fn offer_retry(&mut self) {
        let Some(answer) = self.client.pending_retry.take() else {
            return;
        };
        if retry_slot(&mut self.client.title.0).is_none() {
            return;
        }
        let line = match answer {
            Retry::Wait(line) => line,
            Retry::Now(server) => {
                self.client.retry_due = Some(server);
                String::new()
            }
            // An answer this engine doesn't know: no offer.
            _ => return,
        };
        if let Some(server) = &self.client.last_connection {
            let address = sunrise_client_lib::net::shown_address(&server.address);
            log::info!("Sunrise: offering to connect to {address} again: {line}");
        }
        if let Some(slot) = retry_slot(&mut self.client.title.0) {
            *slot = Some(line);
        }
    }
    /// The server this window connected to last over the network: while connected, and after a
    /// lost connection while the Disconnected screen may connect there again.
    pub(in crate::game::client_side) fn server_address(&self) -> Option<&str> {
        let server = self.client.last_connection.as_ref();
        server.map(|server| server.address.as_str())
    }
    /// That server was found on the local network.
    pub(in crate::game::client_side) fn server_is_lan(&self) -> bool {
        self.client
            .last_connection
            .as_ref()
            .is_some_and(|server| server.lan)
    }
    pub fn set_registries(&mut self, registries: Arc<Dynamic>) -> Result<()> {
        let entries: Vec<(&str, serde_json::Value)> = registries
            .registries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter_map(|(name, data)| {
                Some((
                    name.as_str(),
                    sunrise_game_data::scoreboard::tag_to_json(data.as_ref()?),
                ))
            })
            .collect();
        let enchantments = self
            .stations()
            .enchantments
            .with_entries(entries.iter().map(|(n, v)| (*n, v)))
            .map_err(|error| format!("refused enchantment registry: {error}"))?
            .map(Arc::new);
        let world_biomes = if self.client.integrated.is_none() {
            Some(
                registries
                    .registries
                    .get(BIOME_REGISTRY)
                    .map(|list| client_biomes(self.common.res.biomes(), list))
                    .transpose()
                    .map_err(|error| format!("refused biome registry: {error}"))?
                    .flatten()
                    .map(Arc::new)
                    .unwrap_or_else(|| self.common.res.biomes().clone()),
            )
        } else {
            None
        };
        // The trims render from the registries the server sent, in its sync order.
        self.client
            .armor_stand_assets
            .0
            .trims
            .set_synced(&registries)
            .map_err(|error| format!("refused trim registry: {error}"))?;
        // Bind a per-session biome pair so client biome-color mods never mutate `pack.context`.
        if let Some(table) = world_biomes {
            self.common.world_biomes_base = Some(table.clone());
            self.common.world_biomes = Some(table);
            let context = self.worker_context();
            self.client
                .streaming
                .reload_resources(&mut self.common.world, context);
        }
        let biomes: Vec<u16> = registries
            .registries
            .get(BIOME_REGISTRY)
            .map(|list| {
                list.iter()
                    .map(|(name, _)| world_biomes!(self).id(name))
                    .collect()
            })
            .unwrap_or_default();
        let variants = Arc::new(
            sunrise_sim::mobs::variants::VariantTables::from_synced(
                &registries,
                self.common.res.variants(),
            )
            .map_err(|error| format!("refused variant registries: {error}"))?,
        );
        if let Some(r) = self.client.remote.as_mut() {
            r.enchantments = enchantments;
            let bits = ceil_log2(biomes.len());
            let column = r.chunk_format.layout();
            r.chunk_format =
                Arc::new(ChunkFormat::new(&r.dimension, biomes, bits).in_column(column));
            r.registries = registries;
            r.variants = variants;
            r.resolve_clocks();
            r.level_environments.clear();
            self.install_server_items();
            self.reset_creative_contents();
            self.refresh_creative_search_tags();
        }
        Ok(())
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_hooks::build_creative_mode_tab_contents::GameHooks as _;
