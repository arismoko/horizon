//! A server's workstation menus on this client: the server opens the menu
//! (`ClientboundOpenScreenPacket` with the station's `MenuType`), the
//! station screen draws it, and its clicks and buttons are predicted on the
//! menu engine (`Game::menu_session`) and sent, as
//! `MultiPlayerGameMode.handleContainerInput` and `handleInventoryButtonClick`
//! do. The menu's client-side `slotsChanged` follows the server's slots, and
//! its data slots come in `ClientboundContainerSetDataPacket`.
use super::{Screen, stations::Station};
use sunrise_gameplay_common::game::remote::inventory::remote_station_kind;
use sunrise_gameplay_common::game::stations_common::{INPUT, Kind};
use sunrise_protocol::packets::play::{self, Serverbound as S};

impl crate::game::ClientGame<'_> {
    /// The server opened workstation menu `menu` in window `container`.
    pub fn remote_open_station(
        &mut self,
        container: i32,
        menu: String,
        kind: Kind,
        title: sunrise_game_data::text::Text,
        pos: [i32; 3],
    ) {
        self.remote_set_menu(Some((container, menu)), Some(title));
        self.common.craft_size = 2;
        self.common.container = None;
        self.open_station_client(Station::new_kind(kind, pos));
        self.set_screen(Screen::Inventory);
        self.capture(false);
    }

    /// Window `container` is the server's workstation menu, open here.
    pub(in crate::game::client_side) fn remote_station_open(&self, container: i32) -> bool {
        self.common.station.is_some()
            && self
                .client
                .remote
                .as_ref()
                .and_then(|r| r.menu.as_ref())
                .is_some_and(|(id, name)| *id == container && remote_station_kind(name).is_some())
    }

    /// The server set station slot `internal`: the client menu's
    /// `Slot.set` runs its container's `setChanged`, so the loom lists the
    /// patterns its inputs allow and the stonecutter the recipes of its input.
    pub(in crate::game::client_side) fn remote_station_slot_changed(&mut self, internal: usize) {
        if !self.common.station.as_ref().is_some_and(|s| {
            matches!(
                s.kind,
                Kind::Loom
                    | Kind::Stonecutter
                    | Kind::Grindstone
                    | Kind::Anvil
                    | Kind::Smithing
                    | Kind::Cartography
            )
        }) {
            return;
        }
        if self
            .common
            .station
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Anvil)
        {
            self.anvil_menu_session(|game, menu, to_menu| {
                let i = to_menu(internal)?;
                let stack = menu.slot(i).cloned();
                menu.track_results(true);
                {
                    let env = sunrise_sim::menu::Env {
                        enchantments: game.item_enchantments(),
                        ..game.menu_env()
                    };
                    menu.set_slot(&env, i, stack);
                }
                loop {
                    game.deliver_client_anvil_update(menu);
                    let env = sunrise_sim::menu::Env {
                        enchantments: game.item_enchantments(),
                        ..game.menu_env()
                    };
                    if !menu.advance_result_notification(&env) {
                        break;
                    }
                }
                menu.track_results(false);
                Some(true)
            });
        } else {
            self.menu_session(|menu, env, to_menu| {
                let i = to_menu(internal)?;
                let stack = menu.slot(i).cloned();
                menu.set_slot(env, i, stack);
                Some(true)
            });
        }
        if internal == INPUT
            && let Some((s, ui)) = self.station_and_ui_mut()
            && s.kind == Kind::Stonecutter
        {
            // The client menu's `Slot.set` on the input runs the screen's
            // listener (`StonecutterMenu.container.setChanged`).
            crate::game::client_side::stations_client::stonecutter_changed(ui);
        }
        // AnvilScreen.slotChanged: the box shows the new input's name, which
        // its responder sends.
        self.remote_anvil_rename();
    }

    /// `AbstractContainerMenu.setData` from the server.
    pub(in crate::game::client_side) fn remote_station_data(&mut self, id: usize, value: i16) {
        let Some(s) = self.common.station.as_mut() else {
            return;
        };
        match (s.kind, id) {
            // LoomMenu.selectedBannerPatternIndex.
            (Kind::Loom, 0) => self.client.station_loom.selected = usize::try_from(value).ok(),
            // StonecutterMenu.selectedRecipeIndex.
            (Kind::Stonecutter, 0) => s.selected = usize::try_from(value).ok(),
            // AnvilMenu.cost. A cost outside this short is corrected by
            // `remote_anvil_wide_cost`, which arrives after this packet.
            (Kind::Anvil, 0) => s.anvil.cost = i32::from(value),
            // EnchantmentMenu: costs, the seed (the screen's glyph names),
            // the clues' enchantments and levels.
            (Kind::Enchanting, 0..3) => s.offers.costs[id] = i32::from(value),
            (Kind::Enchanting, 3) => self.common.level.xp_seed = i32::from(value),
            (Kind::Enchanting, 4..7) => {
                let name = self.client.remote.as_ref().and_then(|r| {
                    r.registries
                        .name("minecraft:enchantment", i32::from(value))
                        .map(std::sync::Arc::<str>::from)
                });
                let level = s.offers.clues[id - 4].as_ref().map_or(1, |(_, l)| *l);
                s.offers.clues[id - 4] = name.map(|n| (n, level));
            }
            (Kind::Enchanting, 7..10) => {
                if let Some((_, level)) = s.offers.clues[id - 7].as_mut() {
                    *level = value.max(0) as u32;
                }
            }
            // BeaconMenu: the levels, then the powers (BeaconScreen's
            // dataChanged shows the menu's).
            (Kind::Beacon, 0) => {
                debug_assert!(
                    self.client.station_ui.is_some(),
                    "open client station missing StationUi"
                );
                if let Some(ui) = self.client.station_ui.as_mut() {
                    ui.beacon_levels = Some(i32::from(value));
                }
            }
            (Kind::Beacon, 1 | 2) => {
                let effect = self.remote_effect_id(i32::from(value) - 1);
                let index =
                    effect.and_then(|e| super::beacon::EFFECTS.iter().position(|(n, _)| *n == e));
                if let Some(s) = self.common.station.as_mut() {
                    s.beacon[id - 1] = index;
                }
            }
            _ => {}
        }
    }

    /// `sunrise:anvil_cost`, after the anvil's `ContainerSetData` short: the
    /// signed level cost when that short cannot hold it.
    pub(in crate::game) fn remote_anvil_wide_cost(&mut self, data: &[u8]) {
        let Ok(cost) = sunrise_protocol::anvil_cost::decode(data) else {
            log::debug!("Sunrise: a bad anvil cost payload");
            return;
        };
        if !self.remote_station_open(cost.window) {
            return;
        }
        if let Some(s) = self.common.station.as_mut()
            && s.kind == Kind::Anvil
        {
            s.anvil.cost = cost.cost;
        }
    }

    /// `AnvilScreen.onNameChanged` on a server: the rename box's text (blank
    /// when it is the input's own name) goes to the server when the menu's
    /// `setItemName` takes it.
    pub(in crate::game::client_side) fn remote_anvil_rename(&mut self) {
        if self.client.remote.is_none() {
            return;
        }
        let Some((s, ui)) = self.station_and_ui() else {
            return;
        };
        if s.kind != Kind::Anvil || s.inputs[0].is_none() {
            return;
        }
        let name = self.common.anvil_item_name(s);
        if ui.anvil_sent.as_deref() == Some(&name) {
            return;
        }
        if let Some((_, ui)) = self.station_and_ui_mut() {
            ui.anvil_sent = Some(name.clone());
        }
        self.send(S::RenameItem(play::RenameItem { name }));
    }

    /// `MultiPlayerGameMode.handleInventoryButtonClick`: the screen already
    /// ran `clickMenuButton` on its own menu.
    pub(in crate::game::client_side) fn remote_station_button(&self, button: i32) {
        if let Some((container, _)) = self.client.remote.as_ref().and_then(|r| r.menu.as_ref())
            && self.remote_station_open(*container)
        {
            let _ = self.remote_button_click(button);
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
