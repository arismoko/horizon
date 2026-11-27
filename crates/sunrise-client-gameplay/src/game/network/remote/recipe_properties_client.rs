use crate::game::client_side::launch_options_client::is_client_start;
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Minecraft.disconnect's `ClientPlayerNetworkEvent.LoggingOut`: on
    /// leaving a world, starting a new world or server connection, and
    /// quitting the game (including from the title screen). Fires before
    /// teardown with the live local player handle and game mode when present,
    /// or `(None, None)` on no-player transitions; once per connection,
    /// whichever way out runs first.
    pub fn client_logging_out(&mut self) {
        if !is_client_start(&self.common.options) {
            return;
        }
        if let Some(remote) = self.client.remote.as_mut() {
            if remote.logged_out {
                return;
            }
            remote.logged_out = true;
            remote.camera = None;
            remote.detached_camera = None;
        }
        let player = self
            .client
            .remote
            .as_ref()
            .filter(|remote| remote.has_level())
            .and_then(|_| self.local_player());
        let game_mode = player.map(|_| self.common.mode);
        self.clear_spectated_entity_post_effect();
        self.reset_client_item_use();
        // Camera.reset clears initialization while keeping the last aligned position.
        self.client.view.initialized = false;
        self.native_client_player_logging_out(player, game_mode);
        if self.client.remote.is_none() {
            return;
        }
        if self.client.mod_screen.0.is_some() {
            self.remote_close_local();
        }
        self.unload_client_level();
        self.common.native_lifecycle.begin_close();
        self.native_lifecycle_world_closed();
        self.common.recipe_tables.server_properties = Default::default();
        self.common.recipe_tables.server_stations = None;
        self.refresh_recipe_property_context();
    }
    /// `Minecraft.setLevel` / `clearClientLevel`: once per installed level,
    /// while it is still the active level and its chunks and handles resolve.
    pub(super) fn unload_client_level(&mut self) {
        if let Some(dimension) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.heard_level.take())
        {
            self.native_client_level_unload(&dimension);
        }
    }
    pub(in crate::game::client_side) fn refresh_remote_stonecutter(&mut self) {
        self.common.recipe_tables.server_stations = self.client.remote.as_ref().map(|remote| {
            let item = |id: i32| {
                remote
                    .builtin
                    .get("minecraft:item")?
                    .name(id)
                    .map(str::to_owned)
            };
            let category = |_: i32| None;
            let tag = |name: &str| {
                let name = sunrise_sim::crafting::qualified(name);
                remote
                    .registries
                    .tag("minecraft:item", &name)
                    .into_iter()
                    .flatten()
                    .filter_map(|&id| item(id))
                    .collect()
            };
            let names = sunrise_sim::recipe_book::PacketNames {
                item: &item,
                category: &category,
                component: &|_| None,
                tag: &tag,
            };
            self.stations()
                .recipes
                .with_stonecutting(names.stonecutting(
                    &self.common.recipe_tables.server_properties.stonecutter,
                    |stack| self.decode_remote_stack(stack),
                ))
        });
        if let Some((station, ui)) = self.station_and_ui_mut()
            && station.kind == sunrise_gameplay_common::game::stations_common::Kind::Stonecutter
        {
            // The table and the menu's cached indices are one generation. Rebuild even
            // when the input stack has not changed since the previous packet.
            crate::game::client_side::stations_client::reset_stonecutter_generation(station, ui);
            self.remote_station_slot_changed(sunrise_gameplay_common::game::stations_common::INPUT);
        }
    }
    pub(in crate::game::client_side) fn remote_recipe_properties(
        &mut self,
        packet: sunrise_protocol::packets::recipes::UpdateRecipes,
    ) {
        self.common.recipe_tables.server_properties.replace(packet);
        self.refresh_remote_stonecutter();
        self.refresh_recipe_property_context();
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
