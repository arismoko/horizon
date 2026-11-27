use super::*;
use sunrise_gameplay_common::game::{
    api_values::{project_stack, stack_read_metadata},
    native_hooks::item_smelted::GameHooks as _,
};

impl crate::game::ClientGame<'_> {
    /// `quickMoveStack` of the open plain menu for a shift-click.
    #[allow(dead_code)]
    pub(in crate::game) fn inventory_quick_move(&mut self, slot: usize) -> bool {
        self.click_open_inventory(Some(slot), 0, sunrise_sim::menu::Input::QuickMove)
    }
    /// The open menu as the verified engine (`sunrise_client::menu`): the
    /// game's slots are copied in, `run` acts on it, and the slots, carried
    /// stack, menu data and drops are copied back when it returns Some(true)
    /// (Some(false): handled, nothing kept; None: not handled). False when the
    /// open screen is not one the engine models or `run` did not handle it.
    pub fn menu_session(
        &mut self,
        run: impl FnOnce(
            &mut sunrise_sim::menu::Menu,
            &sunrise_sim::menu::Env,
            &dyn Fn(usize) -> Option<usize>,
        ) -> Option<bool>,
    ) -> bool {
        self.menu_session_with_game(|_, menu, env, to_menu| run(menu, env, to_menu))
    }
    /// A sound a menu click made the player play (a bundle's insert or
    /// remove): `Entity.playSound` at the player.
    pub(in crate::game) fn player_menu_sound(&mut self, sound: &str) {
        if self.common.player.is_none() {
            return;
        }
        let (volume, pitch) = if sound == sunrise_items::bundle::INSERT_FAIL {
            (1.0, 1.0)
        } else {
            (0.8, 0.8 + self.common.world.random.next_float() * 0.4)
        };
        self.play_client_player_sound(sound, volume, pitch);
    }
    pub(in crate::game) fn creative_throw_carrying(&mut self, slot: usize, button: i32) {
        let Some(old) = self
            .slot_mut(slot)
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
        else {
            return;
        };
        let count = if button == 0 {
            1
        } else if self.creative_inventory_open() {
            let max = self.items().get(&old.item).map_or(64, |i| i.max_stack);
            old.count.min(max)
        } else {
            old.count
        };
        let left = old.count - count;
        *self.slot_mut(slot) = (left > 0).then(|| ItemStack {
            count: left,
            ..old.clone()
        });
        self.apply_equipment();
        self.creative_screen_drop(ItemStack { count, ..old });
        self.send_creative_slots();
    }
    /// Like `menu_session_with_game` for a server's open anvil menu, releasing `Env` between
    /// steps so `run` can dispatch live `anvil_update` hooks at every `AnvilMenu.createResult`
    /// without re-running `station_changed` once at the end.
    pub(in crate::game::client_side) fn anvil_menu_session(
        &mut self,
        run: impl FnOnce(
            &mut Self,
            &mut sunrise_sim::menu::Menu,
            &dyn Fn(usize) -> Option<usize>,
        ) -> Option<bool>,
    ) -> bool {
        use sunrise_sim::menu::{Env, Kind, Menu};
        if !self
            .common
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Anvil)
            || self.common.is_logical_server()
            || self.client.remote.is_none()
        {
            return false;
        }
        let mut game_of: Vec<Option<usize>> = vec![
            Some(stations::INPUT),
            Some(stations::INPUT + 1),
            Some(stations::RESULT),
        ];
        game_of.extend((9..36).chain(0..9).map(Some));
        let to_menu = |g: usize| game_of.iter().position(|x| *x == Some(g));
        let mut menu = Menu::new(Kind::Anvil);
        if self.client.remote.is_some() {
            menu.set_drop_spam_throttler(self.client.drop_spam_throttler);
        }
        // Prediction must dispatch the result update between the two input writes in onTake.
        menu.anvil_hook = true;
        menu.set_registry_id(self.remote_menu_type());
        let before: Vec<Option<ItemStack>> = game_of
            .iter()
            .map(|g| g.and_then(|g| self.slot_mut(g).clone()))
            .collect();
        menu.creative = self.common.creative;
        menu.on_client = true;
        menu.selected = self.common.selected;
        menu.levels = self.common.experience.level;
        menu.anvil.name = self
            .station_and_ui()
            .and_then(|(_, ui)| ui.anvil_sent.clone());
        {
            let env = Env {
                enchantments: self.item_enchantments(),
                ..self.menu_env()
            };
            for (i, stack) in before.iter().enumerate() {
                menu.set_slot(&env, i, stack.clone());
            }
            for (i, stack) in self.common.armor.iter().enumerate() {
                menu.set_player_item(39 - i, stack.clone());
            }
            menu.set_player_item(40, self.common.offhand.clone());
            menu.settle(&env);
        }
        if let Some(s) = self.common.station.as_ref() {
            menu.data[0] = s.anvil.cost;
            menu.anvil.repair_count = s.anvil.repair_item_count_cost;
            menu.anvil.only_renaming = s.anvil.only_renaming;
        }
        menu.carried = self.common.carried.clone();
        menu.drops.clear();
        // The AnvilScreen listens while the click is predicted (its slotChanged).
        menu.anvil.seen_input = before[0].clone();
        menu.anvil.screen = true;
        // Clear any createResult flag raised while populating the scratch menu.
        let _ = menu.anvil_created();
        match run(self, &mut menu, &to_menu) {
            None => return false,
            Some(false) => return true,
            Some(true) => {}
        }
        let _ = self.deliver_client_anvil_update(&mut menu);
        let after: Vec<Option<ItemStack>> = (0..menu.slot_count())
            .map(|i| menu.slot(i).cloned())
            .collect();
        let carried = menu.carried.take();
        for sound in std::mem::take(&mut menu.player_sounds) {
            self.player_menu_sound(sound);
        }
        for (_, stack, _) in std::mem::take(&mut menu.item_crafted) {
            self.tutorial_get_item(Some(&stack));
        }
        for (i, stack) in after.into_iter().enumerate() {
            if let Some(g) = game_of[i] {
                *self.slot_mut(g) = stack;
            }
        }
        self.common.carried = carried;
        self.common.offhand = menu.player_item(40).cloned();
        // The names AnvilScreen.onNameChanged sent while the click ran go out before it.
        for name in std::mem::take(&mut menu.anvil.renamed) {
            if let Some((_, ui)) = self.station_and_ui_mut() {
                ui.anvil_sent = Some(name.clone());
            }
            self.send(sunrise_protocol::packets::play::Serverbound::RenameItem(
                sunrise_protocol::packets::play::RenameItem { name },
            ));
        }
        self.anvil_session_end(&menu);
        self.apply_equipment();
        true
    }
    pub(in crate::game::client_side) fn click_inventory(&mut self, right: bool) -> bool {
        if self.mod_browser_press(right) || self.mount_click(right) {
            return true;
        }
        if self.recipe_book_click(right) {
            return true;
        }
        if self.station_click(right) {
            return true;
        }
        self.client.remote.is_some() && self.remote_click(right)
    }
    /// `AbstractContainerMenu.clicked` on the open inventory, crafting table,
    /// chest, hopper, dispenser or furnace menu, run by the verified menu
    /// engine (`sunrise_client::menu`). `slot` None is outside the window.
    /// False when the open screen is not one the engine models.
    pub fn click_open_inventory(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: sunrise_sim::menu::Input,
    ) -> bool {
        if let Some(slot) = slot
            && slot >= Creative::DISPLAY
            && self.creative_tab_open()
        {
            self.creative_display_click(slot, button, input);
            return true;
        }
        if self.client.screen == Screen::Creative
            && input == sunrise_sim::menu::Input::Throw
            && !self.client.drop_spam_throttler.is_under_threshold()
        {
            return true;
        }
        if let Some(slot) = slot
            && input == sunrise_sim::menu::Input::Throw
            && self.common.carried.is_some()
            && self.client.remote.is_some()
            && ((slot < 9 && self.creative_tab_open()) || self.creative_inventory_open())
        {
            self.creative_throw_carrying(slot, button);
            return true;
        }
        self.menu_ops(&[(slot, button, input)])
    }
    /// A click on an item tab's display slot: the screen's own handling
    /// (`creative::click_display`), never the menu's.
    pub fn creative_display_click(
        &mut self,
        slot: usize,
        button: i32,
        input: sunrise_sim::menu::Input,
    ) {
        use sunrise_sim::menu::DisplayClick;
        if self.client.creative_screen.0.tab().search {
            // CreativeModeInventoryScreen.slotClicked: moveCursorToEnd, then
            // setHighlightPos(0), so typing replaces the whole query.
            let search = &mut self.client.creative_screen.0.search;
            search.move_cursor_to_end(false);
            search.highlight = 0;
        }
        if input == sunrise_sim::menu::Input::Throw
            && !self.client.drop_spam_throttler.is_under_threshold()
        {
            return;
        }
        let shown = self.creative_display_stack(slot);
        let catalog = self
            .common
            .server_items
            .as_deref()
            .unwrap_or_else(|| self.common.res.items());
        let max = |s: &ItemStack| catalog.get(&s.item).map_or(64, |i| i.max_stack);
        match sunrise_sim::menu::click_display(
            &mut self.common.carried,
            shown.as_ref(),
            max,
            button,
            input,
        ) {
            DisplayClick::Hotbar(i, stack) if i < self.common.inventory.len() => {
                self.common.inventory[i] = Some(stack);
            }
            DisplayClick::Offhand(stack) => self.common.offhand = Some(stack),
            // handleCreativeModeItemDrop: on a server the stack goes to it (SetCreativeModeSlot
            // to slot -1), which drops it; a disconnected client has no world to drop into.
            DisplayClick::Drop(stack) if self.client.remote.is_some() => {
                self.creative_screen_drop(stack)
            }
            DisplayClick::Drop(_) => {}
            _ => {}
        }
        // handleCreativeModeItemAdd / broadcastChanges: a swapped-in hotbar slot goes out now.
        if self.client.remote.is_some() {
            self.send_creative_slots();
        }
    }
    /// Deliver `anvil_update` for a client anvil menu whenever `AnvilMenu.createResult` ran
    /// with a left input (matching NeoForge `CommonHooks.onAnvilUpdate` on the client side).
    pub(in crate::game::client_side) fn deliver_client_anvil_update(
        &mut self,
        menu: &mut sunrise_sim::menu::Menu,
    ) -> bool {
        if !menu.anvil_created() || self.common.is_logical_server() || self.client.remote.is_none()
        {
            return false;
        }
        if !mod_api::__private::listened(mod_api::HookId::AnvilUpdate) {
            return false;
        }
        let Some(pos) = self.common.station.as_ref().map(|s| s.pos) else {
            return false;
        };
        let Some((inputs, mut result)) = menu.anvil_update() else {
            return false;
        };
        let (player, entity_id) = (self.native_acting_id(), self.own_entity_id());
        self.api_anvil_update(player, entity_id, pos, inputs, &mut result);
        menu.anvil_updated(result);
        true
    }
    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn hovered_inventory_slot(&self) -> Option<usize> {
        let size = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])?;
        let batch = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        self.slot_at(&batch, self.client.cursor.0, self.client.cursor.1)
    }
    /// Several clicks on one menu instance (a quick craft drag is three).
    pub fn menu_ops(&mut self, ops: &[(Option<usize>, i32, sunrise_sim::menu::Input)]) -> bool {
        use sunrise_sim::menu::Input;
        // BundleMouseActions.onSlotClicked: a Shift click or a swap clears the bundle's selection.
        for &(slot, _, input) in ops {
            if let (Some(slot), Input::QuickMove | Input::Swap) = (slot, input) {
                self.unselect_bundle(slot);
            }
        }
        if !self.common.is_logical_server() && self.client.screen == Screen::Inventory {
            return self.client.remote.is_some() && self.remote_menu_ops(ops);
        }
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        if let Some(done) = self.stacked_menu_ops(ops) {
            // CreativeModeInventoryScreen.slotClicked: the changed slots go out with the click.
            if done && self.client.remote.is_some() && self.client.screen == Screen::Creative {
                self.send_creative_slots();
            }
            return done;
        }
        let ops = ops.to_vec();
        let creative_remote =
            self.client.remote.is_some() && self.client.screen == Screen::Creative;
        let done = self.menu_session_with_game(move |game, menu, env, to_menu| {
            let mut clicks = Vec::new();
            for &(slot, button, input) in &ops {
                let menu_slot = match slot {
                    Some(g) => {
                        if g == 200 && input != Input::QuickMove {
                            return None;
                        }
                        match to_menu(g) {
                            Some(i) => i as i32,
                            None => return None,
                        }
                    }
                    None => -999,
                };
                clicks.push((menu_slot, button, input));
            }
            for (menu_slot, button, input) in clicks {
                let result = if creative_remote {
                    menu.click_with_creative_drop(env, menu_slot, button, input, |stack| {
                        game.try_send_creative_drop_packet(stack)
                    })
                } else {
                    menu.click(env, menu_slot, button, input)
                };
                if result.is_err() {
                    // Vanilla's server throws: nothing of this click is kept.
                    return Some(false);
                }
            }
            Some(true)
        });
        // CreativeModeInventoryScreen.slotClicked: the changed slots go out with the click.
        if done && self.client.remote.is_some() && self.client.screen == Screen::Creative {
            self.send_creative_slots();
        }
        done
    }
    /// Like `menu_session`, with read-only access to connection state before
    /// deciding whether to keep the prediction.
    pub(in crate::game) fn menu_session_with_game(
        &mut self,
        run: impl FnOnce(
            &Self,
            &mut sunrise_sim::menu::Menu,
            &sunrise_sim::menu::Env,
            &dyn Fn(usize) -> Option<usize>,
        ) -> Option<bool>,
    ) -> bool {
        use sunrise_sim::menu::{Env, Kind, Menu};
        #[derive(Clone, Copy)]
        struct Map {
            container: usize,
            base: usize,
            table: bool,
            cells: usize,
            plain: bool,
        }
        impl Map {
            fn main(&self) -> usize {
                match (self.plain, self.table) {
                    (false, _) => self.container,
                    (true, true) => 10,
                    (true, false) => 9,
                }
            }
            fn to_menu(self, g: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match g {
                        200 => Some(0),
                        100..109 if g - 100 < self.cells => Some(1 + g - 100),
                        500..504 if !self.table => Some(5 + g - 500),
                        504 if !self.table => Some(45),
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                } else {
                    match g {
                        g if g >= self.base && g < self.base + self.container => {
                            Some(g - self.base)
                        }
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                }
            }
            fn to_game(self, i: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match i {
                        0 => None,
                        1..=9 if i - 1 < self.cells => Some(100 + i - 1),
                        5..9 if !self.table => Some(500 + i - 5),
                        45 if !self.table => Some(504),
                        _ if i >= main && i < main + 27 => Some(9 + i - main),
                        _ if i >= main + 27 && i < main + 36 => Some(i - main - 27),
                        _ => None,
                    }
                } else if i < self.container {
                    Some(self.base + i)
                } else if i < self.container + 27 {
                    Some(9 + i - self.container)
                } else {
                    Some(i - self.container - 27)
                }
            }
        }
        // Menu slot -> game slot, for every menu slot.
        let mut plain_map = None;
        let crafter_open = matches!(self.container_kind(), Some(BlockEntity::Crafter(_)));
        let loom_open = self
            .common
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Loom);
        let merchant_open = self
            .common
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Merchant);
        let stonecutter_open = self
            .common
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Stonecutter);
        let station_kind = self.common.station.as_ref().map(|s| s.kind);
        let grindstone_open = station_kind == Some(stations::Kind::Grindstone);
        let cartography_open = station_kind == Some(stations::Kind::Cartography);
        let anvil_open = station_kind == Some(stations::Kind::Anvil);
        let smithing_open = station_kind == Some(stations::Kind::Smithing);
        let enchanting_open = station_kind == Some(stations::Kind::Enchanting);
        let beacon_open = station_kind == Some(stations::Kind::Beacon);
        let (kind, game_of): (Kind, Vec<Option<usize>>) = if self.creative_tab_open() {
            // ItemPickerMenu: the display slots (clicked by the screen), then the hotbar.
            let mut v = vec![None; 45];
            v.extend((0..9).map(Some));
            (Kind::ItemPicker, v)
        } else if let Some(columns) = self
            .mount_menu_columns()
            .filter(|_| self.client.remote.is_some() && self.mount_active())
        {
            // A server's HorseInventoryMenu: the saddle, the body armour,
            // the chest, then the inventory.
            let mut v = vec![
                Some(sunrise_gameplay_common::mount_rules::SADDLE),
                Some(sunrise_gameplay_common::mount_rules::ARMOR),
            ];
            v.extend(
                (0..columns * 3).map(|i| Some(sunrise_gameplay_common::mount_rules::CHEST + i)),
            );
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Mount(columns), v)
        } else if merchant_open {
            // MerchantMenu: two payment slots, the result, then the inventory.
            let mut v: Vec<Option<usize>> = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Merchant, v)
        } else if crafter_open {
            // CrafterMenu: the grid, the inventory, then the result.
            let mut v: Vec<Option<usize>> = (300..309).map(Some).collect();
            v.extend((9..36).chain(0..9).map(Some));
            v.push(None);
            (Kind::CrafterBlock, v)
        } else if loom_open {
            // LoomMenu: banner, dye, pattern, result, then the inventory.
            let mut v: Vec<Option<usize>> = (0..3).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Loom, v)
        } else if stonecutter_open {
            // StonecutterMenu: the input, the result, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::RESULT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Stonecutter, v)
        } else if grindstone_open || cartography_open {
            // GrindstoneMenu and CartographyTableMenu: the two inputs, the
            // result, then the inventory.
            let mut v = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            let kind = if grindstone_open {
                Kind::Grindstone
            } else {
                Kind::Cartography
            };
            (kind, v)
        } else if enchanting_open {
            // EnchantmentMenu: the item, the lapis, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::INPUT + 1)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Enchanting, v)
        } else if beacon_open {
            // BeaconMenu: the payment, then the inventory.
            let mut v = vec![Some(stations::INPUT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Beacon, v)
        } else if anvil_open || smithing_open {
            // ItemCombinerMenu: the inputs, the result, then the inventory.
            let inputs = if anvil_open { 2 } else { 3 };
            let mut v: Vec<Option<usize>> =
                (0..inputs).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (
                if anvil_open {
                    Kind::Anvil
                } else {
                    Kind::Smithing
                },
                v,
            )
        } else {
            let (kind, map) = if self.common.container.is_some() {
                let size = self.common.open_container_size();
                let (kind, count) = match self.common.container_kind() {
                    Some(BlockEntity::Chest(_)) => {
                        let shulker = self.common.container.is_some_and(|k| {
                            let id = self.common.world.block(k.0, k.1, k.2);
                            storage::is_shulker_box(&self.common.res.facts().get(id).name)
                        });
                        let kind = match size {
                            5 => Kind::Hopper,
                            9 => Kind::Dispenser,
                            27 if shulker => Kind::Shulker,
                            n => Kind::Chest((n / 9).clamp(1, 6)),
                        };
                        (kind, size)
                    }
                    Some(BlockEntity::Brewing(_)) => (Kind::Brewing, 5),
                    Some(BlockEntity::Mod(_)) => {
                        match self
                            .remote_menu_type()
                            .and_then(sunrise_level::containers::mod_menu_type)
                        {
                            Some(t) => (Kind::Mod(t), t.slots),
                            None => return false,
                        }
                    }
                    Some(BlockEntity::Furnace(f)) => (
                        Kind::Furnace(
                            match f.kind.recipe_type().trim_start_matches("minecraft:") {
                                "blasting" => "blasting",
                                "smoking" => "smoking",
                                _ => "smelting",
                            },
                        ),
                        3,
                    ),
                    _ => return false,
                };
                let map = Map {
                    container: count,
                    base: if matches!(kind, Kind::Mod(_)) {
                        sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE
                    } else {
                        300
                    },
                    table: false,
                    cells: 0,
                    plain: false,
                };
                (kind, map)
            } else if self.common.station.is_none() && !self.mount_active() {
                let table = self.common.craft_size == 3;
                let map = Map {
                    container: 0,
                    base: 300,
                    table,
                    cells: self.common.craft_size * self.common.craft_size,
                    plain: true,
                };
                (
                    if table {
                        Kind::Crafting
                    } else {
                        Kind::Inventory
                    },
                    map,
                )
            } else {
                return false;
            };
            let total = Menu::new(kind).slot_count();
            plain_map = Some(map);
            (kind, (0..total).map(|i| map.to_game(i)).collect())
        };
        // The plain map also takes the crafting result slot (a shift-click on it).
        let to_menu = |g: usize| match plain_map {
            Some(map) => map.to_menu(g),
            None => game_of.iter().position(|x| *x == Some(g)),
        };
        let mut menu = Menu::new(kind);
        if self.client.remote.is_some() {
            menu.set_drop_spam_throttler(self.client.drop_spam_throttler);
        }
        let menu_id = if matches!(kind, Kind::Mount(_)) {
            Some("sunrise:mount_inventory")
        } else if kind == Kind::Inventory {
            Some(sunrise_sim::menu::INVENTORY_MENU)
        } else if kind == Kind::ItemPicker {
            None
        } else {
            self.remote_menu_type()
        };
        menu.set_registry_id(menu_id);
        menu.set_block_pos(self.common.container.map(|(x, y, z)| [x, y, z]));
        menu.brewed_potion_hook = !mod_api::__private::mods().is_empty();
        menu.item_smelted_hook = mod_api::__private::listened(mod_api::HookId::ItemSmelted);
        if matches!(kind, Kind::Mount(_)) {
            menu.mount = self.mount_menu_entity();
        }
        let total = menu.slot_count();
        let before: Vec<Option<ItemStack>> = game_of
            .iter()
            .map(|g| g.and_then(|g| self.slot_mut(g).clone()))
            .collect();
        let carried = self.common.carried.clone();
        let creative = self.common.creative;
        // CreativeModeInventoryScreen clicks the menu itself and sends every
        // changed slot, which the server sets by player.
        let station_menu = self.common.station.as_ref().map(|_| {
            (
                self.client.station_loom.selected.map_or(-1, |i| i as i32),
                self.client.station_loom.patterns.clone(),
            )
        });
        let stone = self
            .common
            .station
            .as_ref()
            .map(|s| (s.selected.map_or(-1, |i| i as i32), s.stone.clone()));
        let crafter_data = match self.common.container_kind() {
            Some(BlockEntity::Crafter(c)) => Some((c.disabled, c.is_triggered())),
            _ => None,
        };
        // The merchant's session, lent to the menu for this click.
        let session = self
            .common
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .map(|m| m.session.clone());
        let menu_can_equip = self.menu_can_equip();
        let env = Env {
            enchantments: self.item_enchantments(),
            trade: None,
            maps: None,
            can_equip: Some(&menu_can_equip),
            ..self.menu_env()
        };
        menu.creative = creative;
        // A client predicts with the server's crafting result, never its own.
        menu.on_client = true;
        menu.selected = self.common.selected;
        menu.levels = self.common.experience.level;
        if anvil_open {
            // The rename box as AnvilScreen.onNameChanged gives it to the menu.
            menu.anvil.name = self
                .station_and_ui()
                .and_then(|(_, ui)| ui.anvil_sent.clone());
        }
        for (i, stack) in before.iter().enumerate() {
            menu.set_slot(&env, i, stack.clone());
        }
        // A number-key swap uses the backing inventory, not the displayed slots.
        // Keep all 36 slots available, including room for a swap's overflow.
        let inventory_omitted = matches!(kind, Kind::Mod(layout) if !layout.player_inventory);
        if inventory_omitted {
            for (slot, stack) in self.common.inventory.iter().take(36).enumerate() {
                menu.set_player_item(slot, stack.clone());
            }
        }
        // Armor and off hand are part of the inventory even when the menu has no slots for them.
        if !game_of.contains(&Some(500)) {
            for (i, stack) in self.common.armor.iter().enumerate() {
                menu.set_player_item(39 - i, stack.clone());
            }
        }
        let offhand_mapped = game_of.contains(&Some(504));
        if !offhand_mapped {
            menu.set_player_item(40, self.common.offhand.clone());
        }
        menu.settle(&env);
        // A server's client menu never works out its crafting result (`on_client`): the crafting
        // menus' result slot (InventoryMenu's and CraftingMenu's slot 0, which `game_of` doesn't
        // map) holds what the server's ContainerSetSlot put there. Another menu's slot 0 is its own.
        if matches!(kind, Kind::Inventory | Kind::Crafting) {
            menu.set_slot(&env, 0, self.craft_result());
        }
        if kind == Kind::Loom
            && let Some((selected, patterns)) = station_menu
        {
            // The menu's state between clicks: what set_slot recomputed is replaced.
            menu.data[0] = selected;
            menu.loom_patterns = patterns;
            menu.set_slot(&env, 3, before[3].clone());
        }
        if kind == Kind::Stonecutter
            && let Some((selected, state)) = stone
        {
            // The same: the list and selection between clicks, then the result.
            menu.data[0] = selected;
            menu.stone = state;
            menu.set_slot(&env, 1, before[1].clone());
        }
        if anvil_open && let Some(s) = self.common.station.as_ref() {
            // The same: the cost, material cost and rename flag between clicks, as
            // the station's createResult and the mods' anvil_update left them (the
            // result came with the slots), so a take is predicted as the server makes it.
            menu.data[0] = s.anvil.cost;
            menu.anvil.repair_count = s.anvil.repair_item_count_cost;
            menu.anvil.only_renaming = s.anvil.only_renaming;
        }
        if kind == Kind::Enchanting
            && let Some(s) = self.common.station.as_ref()
        {
            // The offers between clicks (what copying the slots in flagged is not a change).
            menu.enchant_changed = false;
            for i in 0..3 {
                menu.data[i] = s.offers.costs[i];
                menu.data[7 + i] = s.offers.clues[i].as_ref().map_or(-1, |(_, l)| *l as i32);
            }
            menu.data[3] = self.common.level.xp_seed;
        }
        if let Some((disabled, triggered)) = crafter_data {
            for (i, d) in disabled.iter().enumerate() {
                menu.data[i] = i32::from(*d);
            }
            menu.data[9] = i32::from(triggered);
        }
        menu.carried = carried;
        menu.drops.clear();
        if kind == Kind::Merchant {
            // Attached after the slots were set: setting them is not a click.
            menu.merchant = session;
        }
        if anvil_open {
            // The AnvilScreen listens while the click is predicted (its slotChanged).
            menu.anvil.seen_input = before[0].clone();
            menu.anvil.screen = true;
            menu.anvil_hook = true;
        }
        // Clear any createResult flag raised while populating the menu before the click.
        let _ = menu.anvil_created();
        let outcome = run(self, &mut menu, &env, &to_menu);
        // Any creative-screen drop sent live inside `run` already updated `menu`'s
        // throttler; keep `self.drop_spam_throttler` in step even if a later click failed.
        if self.client.remote.is_some()
            && self.client.screen == Screen::Creative
            && let Some(throttle) = menu.drop_spam_throttler()
        {
            self.client.drop_spam_throttler = throttle;
        }
        match outcome {
            None => return false,
            Some(false) => return true,
            Some(true) => {}
        }
        // On a client the clicks only predict: the server drops, crafts and makes maps itself.
        if let Some(s) = menu.merchant.take()
            && let Some(m) = self
                .common
                .station
                .as_mut()
                .and_then(|s| s.merchant.as_mut())
        {
            m.session = s;
        }
        let after: Vec<Option<ItemStack>> = (0..total).map(|i| menu.slot(i).cloned()).collect();
        let carried = menu.carried.take();
        let dropped = std::mem::take(&mut menu.dropped);
        // A bundle's sounds are the player's own (`Entity.playSound`): the
        // client plays them as it clicks, predicting or not.
        for sound in std::mem::take(&mut menu.player_sounds) {
            self.player_menu_sound(sound);
        }
        let item_crafted = std::mem::take(&mut menu.item_crafted);
        let item_smelted = std::mem::take(&mut menu.item_smelted);
        for (_, stack, _) in &item_crafted {
            self.tutorial_get_item(Some(stack));
        }
        for (i, stack) in after.into_iter().enumerate() {
            if let Some(g) = game_of[i] {
                *self.slot_mut(g) = stack;
            }
        }
        if inventory_omitted {
            for (slot, stack) in self.common.inventory.iter_mut().take(36).enumerate() {
                *stack = menu.player_item(slot).cloned();
            }
        }
        self.common.carried = carried;
        if !offhand_mapped {
            self.common.offhand = menu.player_item(40).cloned();
        }
        if self.client.screen == Screen::Creative {
            // `Player.drop` swings even when the creative packet is suppressed. Creative-screen
            // THROW packets already went out live inside `click_with_creative_drop`, before the
            // changed-slot sync.
            for _ in dropped {
                self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
            }
        }
        if kind == Kind::Loom {
            if self.common.station.is_some() {
                self.client.station_loom.selected = usize::try_from(menu.data[0]).ok();
                self.client.station_loom.patterns = std::mem::take(&mut menu.loom_patterns);
            }
            self.loom_screen_changed();
        } else if kind == Kind::Stonecutter {
            let input_changed = before[0] != menu.slot(0).cloned();
            if let Some((s, ui)) = self.station_and_ui_mut() {
                s.selected = usize::try_from(menu.data[0]).ok();
                s.stone = std::mem::take(&mut menu.stone);
                if input_changed {
                    crate::game::client_side::stations_client::stonecutter_changed(ui);
                }
            }
        }
        if kind == Kind::Anvil {
            // The names AnvilScreen.onNameChanged sent while the click ran go out before it.
            for name in std::mem::take(&mut menu.anvil.renamed) {
                if let Some((_, ui)) = self.station_and_ui_mut() {
                    ui.anvil_sent = Some(name.clone());
                }
                self.send(sunrise_protocol::packets::play::Serverbound::RenameItem(
                    sunrise_protocol::packets::play::RenameItem { name },
                ));
            }
        }
        if kind == Kind::Anvil || kind == Kind::Smithing {
            let anvil_created = menu.anvil_created();
            self.combiner_session_end(&menu, anvil_created, true);
        }
        if kind == Kind::Enchanting && (menu.enchant_changed || before[0] != menu.slot(0).cloned())
        {
            // EnchantmentMenu.slotsChanged: new offers for the item.
            self.station_changed();
        }
        if kind == Kind::CrafterBlock {
            self.apply_crafter_slot_states(&menu);
        }
        self.apply_equipment();
        if !item_smelted.is_empty()
            && mod_api::__private::listened(mod_api::HookId::ItemSmelted)
            && let Some(player) = self.local_player()
        {
            for (stack, amount_removed) in &item_smelted {
                let Ok(metadata) = stack_read_metadata(stack) else {
                    continue;
                };
                let project = |metadata| project_stack(stack, metadata).ok();
                let stack = mod_api::ItemStackView::resident(metadata, &project);
                self.native_item_smelted(mod_api::__private::ItemSmeltedCall {
                    player,
                    stack,
                    amount_removed: *amount_removed,
                });
            }
        }
        true
    }
    /// A screen closed or lost its pointer: discard the unfinished click gesture.
    pub(in crate::game) fn reset_inventory_clicks(&mut self) {
        self.client.inv_clicks.cancel_drag();
        self.client.inv_drag = None;
        self.client.last_inv_click = None;
    }
    /// Run one client anvil click with live `anvil_update` dispatch at every `createResult`.
    pub(in crate::game::client_side) fn run_client_anvil_click(
        &mut self,
        menu: &mut sunrise_sim::menu::Menu,
        slot: i32,
        button: i32,
        input: sunrise_sim::menu::Input,
    ) -> std::result::Result<(), &'static str> {
        use sunrise_sim::menu::Env;
        menu.track_results(true);
        let mut rest = {
            let env = Env {
                enchantments: self.item_enchantments(),
                ..self.menu_env()
            };
            menu.start_click(&env, slot, button, input)
        }?;
        loop {
            self.deliver_client_anvil_update(menu);
            if menu.anvil_craft.take().is_some() {
                menu.record_anvil_charge();
                menu.anvil_take_material(&Env {
                    enchantments: self.item_enchantments(),
                    ..self.menu_env()
                });
                self.deliver_client_anvil_update(menu);
                menu.anvil_take_input(&Env {
                    enchantments: self.item_enchantments(),
                    ..self.menu_env()
                });
                self.deliver_client_anvil_update(menu);
            }
            let Some(step) = rest else {
                break;
            };
            let env = Env {
                enchantments: self.item_enchantments(),
                ..self.menu_env()
            };
            rest = menu.resume_click(&env, step);
        }
        menu.track_results(false);
        Ok(())
    }
}

use sunrise_gameplay_common::game::api_values_common::ApiValuesCommonGameExt1 as _;

use sunrise_gameplay_common::game::equip_answers::EquipAnswersGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::menu_clicks::MenuClicksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

use sunrise_gameplay_common::game::stations_common::StationsCommonGameExt1 as _;
