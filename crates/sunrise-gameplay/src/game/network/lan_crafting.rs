//! Crafting for LAN guests: the inventory's 2x2 grid (window 0) and the
//! crafting table (`minecraft:crafting`) run the host's menu engine
//! (`sunrise_client::menu`, `Kind::Inventory` / `Kind::Crafting`) over the
//! guest's inventory, with vanilla's packets: OpenScreen, container clicks
//! and their corrections, and the grid back into the inventory when the menu
//! closes (`AbstractContainerMenu.clearContainer`).
use sunrise_gameplay_common::game::lan_menu_data::{FurnaceMenuEffects, GuestMenuEffects};
use sunrise_gameplay_common::game::{
    api_values::{project_stack, stack_read_metadata},
    lan_actions::RECIPE_CHECK_COST,
    transactions::{After, decision::MenuOwner},
};
use sunrise_items::items::{ItemStack, Items};
use sunrise_level::containers::{BlockEntity, ModMenuType};
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::lan::recipes;
use sunrise_sim::{
    furnace::FurnaceCooking,
    lan::{
        guest::Guest,
        menu::{LinkedSlots, ModCraft, Slots, linked_slot_count, write_links},
    },
    menu::{
        Env, Input, Kind, Menu,
        place::{PostPlace, TransferCheck, TransferError},
    },
};

/// `AbstractContainerMenu.dropOrPlaceInInventory`: killed or disconnected players
/// drop inputs, while a live close or a dimension change returns them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::game) enum MenuRemoval {
    Live,
    ChangedDimension,
    Killed,
    Disconnected,
}

impl MenuRemoval {
    pub(in crate::game) fn drops_items(self) -> bool {
        matches!(self, Self::Killed | Self::Disconnected)
    }
}

#[derive(Default)]
pub(in crate::game) struct LinkScratch {
    asked: Vec<[i32; 3]>,
    seen: std::collections::HashSet<(i32, i32, i32)>,
    links: Vec<(i32, i32, i32)>,
}

#[derive(Clone, Copy)]
pub(in crate::game) enum RecipePlacementIntent<'a> {
    Vanilla {
        container: i32,
        display: i32,
        use_max: bool,
    },
    ModBook {
        catalog: &'a sunrise_sim::recipe_book::placement_sync::CatalogIdentity,
        display: &'a sunrise_sim::recipe_book::placement_sync::DisplayId,
        use_max: bool,
    },
}

fn guest_open_window(host: &super::lan::LanHost, conn: u32) -> i32 {
    if let Some(m) = host.hub.guest(conn).and_then(|g| g.menu.as_ref()) {
        return m.window;
    }
    if let Some((window, _, _)) = super::native_menus::open_menu_of(host, conn) {
        return window;
    }
    host.menus
        .get(&conn)
        .and_then(|menus| {
            menus
                .crafting
                .table
                .as_ref()
                .map(|(window, ..)| *window)
                .or_else(|| menus.station.as_ref().map(|station| station.window))
        })
        .unwrap_or(0)
}

/// The guest's menus the game runs: kept between clicks, so a drag (quick
/// craft) spans them as in vanilla.
#[derive(Default)]
pub struct GuestCrafting {
    /// The inventory menu; its grid lives in the hub's guest.
    pub inventory: MenuOwner<Option<Box<Menu>>>,
    /// The open crafting table: its window, state id, menu and block.
    pub table: MenuOwner<Option<(i32, i32, Box<Menu>, [i32; 3])>>,
    /// The open furnace's or brewing stand's window and menu (`AbstractFurnaceMenu`,
    /// `BrewingStandMenu`); its slots live in the block entity and the hub's
    /// container menu keeps the client in sync.
    pub machine: MenuOwner<Option<(i32, Box<Menu>)>>,
}

/// ServerboundContainerClickPacket's input.
pub(in crate::game) fn input(id: i32) -> Input {
    match id {
        0 => Input::Pickup,
        1 => Input::QuickMove,
        2 => Input::Swap,
        3 => Input::Clone,
        4 => Input::Throw,
        5 => Input::QuickCraft,
        _ => Input::PickupAll,
    }
}

/// The armor's player-container slots, head to feet (`Inventory` 39..36).
const ARMOR: [usize; 4] = [39, 38, 37, 36];
const OFFHAND: usize = 40;

/// The guest's inventory, armor, off hand, selection and carried stack into `menu`.
pub(in crate::game) fn load_player(menu: &mut Menu, g: &Guest) {
    for i in 0..36 {
        menu.set_player_item(i, g.inventory[i].clone());
    }
    for (k, slot) in ARMOR.into_iter().enumerate() {
        menu.set_player_item(slot, g.armor[k].clone());
    }
    menu.set_player_item(OFFHAND, g.offhand.clone());
    menu.selected = g.selected;
    menu.creative = g.creative();
    menu.levels = g.xp_level;
    menu.carried = g.carried.clone();
}

/// The menu engine's kind for a guest's machine or block-backed mod menu.
pub(super) fn machine_menu_kind(
    entity: &BlockEntity,
    mod_type: Option<&'static ModMenuType>,
) -> Option<Kind> {
    Some(match entity {
        BlockEntity::Furnace(f) => Kind::Furnace(match f.kind.recipe_type() {
            "minecraft:blasting" => "blasting",
            "minecraft:smoking" => "smoking",
            _ => "smelting",
        }),
        BlockEntity::Brewing(_) => Kind::Brewing,
        BlockEntity::Crafter(_) => Kind::CrafterBlock,
        BlockEntity::Mod(_) => Kind::Mod(mod_type?),
        _ => return None,
    })
}

/// The machine's working slot storage for an edit, including any empty mod menu slots.
pub(in crate::game) fn machine_menu(
    entity: &BlockEntity,
    mod_type: Option<&'static ModMenuType>,
) -> Option<(Kind, Vec<Option<ItemStack>>)> {
    let kind = machine_menu_kind(entity, mod_type)?;
    let mut items = entity.slots().to_vec();
    if let Kind::Mod(layout) = kind {
        items.resize(layout.slots, None);
    }
    Some((kind, items))
}

/// `load_player` back.
pub(in crate::game) fn store_player(menu: &mut Menu, g: &mut Guest) {
    for i in 0..36 {
        g.inventory[i] = menu.player_item(i).cloned();
    }
    for (k, slot) in ARMOR.into_iter().enumerate() {
        g.armor[k] = menu.player_item(slot).cloned();
    }
    g.offhand = menu.player_item(OFFHAND).cloned();
    g.carried = menu.carried.take();
}

/// Inventory.placeItemBackInInventory for each stack: what doesn't fit is
/// left for Player.drop (creative players keep it too).
fn place_back(menu: &mut Menu, items: &Items, stacks: impl IntoIterator<Item = ItemStack>) {
    menu.creative = false;
    for mut stack in stacks {
        menu.give(items, &mut stack);
        if stack.count > 0 {
            menu.dropped.push(stack);
        }
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn guest_slot_env<'s>(
        &'s self,
        guest: &'s Guest,
        parts: &'s dyn sunrise_sim::mobs::equip::CanEquip,
    ) -> Env<'s> {
        Env {
            player: Some(guest.uuid),
            player_entity_id: Some(guest.entity_id),
            limited_crafting: self.server.lan.as_ref().unwrap().hub.limited_crafting,
            can_equip: Some(parts),
            known_recipes: Some(if self.server.lan_acting == Some(guest.conn) {
                &self.common.level.recipe_book
            } else {
                &guest.recipe_book
            }),
            ..self.menu_env()
        }
    }

    /// Runs `op` on the guest's menu with their inventory copied in and back.
    pub fn guest_menu_op<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        self.guest_menu_op_with(gi, menu, None, op)
    }

    /// `guest_menu_op` with the mods' answer to a decided pickup, when there is one.
    pub(in crate::game) fn guest_menu_op_with<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        stacked: Option<&sunrise_sim::menu::StackedOutcome>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        let result = self.guest_menu_edit(gi, menu, stacked, op)?;
        let conn = self.server.lan.as_ref()?.hub.guests[gi].conn;
        self.publish_guest_menu_effects(conn, GuestMenuEffects::take(menu));
        Some(result)
    }

    /// Run one edit with the guest's rules and storage, leaving effects in the menu until its
    /// caller accepts the action. No `Env` survives this call.
    pub(in crate::game) fn guest_menu_edit<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        stacked: Option<&sunrise_sim::menu::StackedOutcome>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        self.server.lan.as_ref()?;
        // Crafting and cartography read and derive this level's saved maps.
        let maps = matches!(
            menu.kind(),
            Kind::Cartography | Kind::Crafting | Kind::Inventory
        )
        .then(|| {
            let g = &self.server.lan.as_ref().unwrap().hub.guests[gi];
            (0..menu.slot_count())
                .filter_map(|s| menu.slot(s).cloned())
                .chain(g.inventory.iter().flatten().cloned())
                .chain(g.carried.clone())
                .collect::<Vec<_>>()
        })
        .filter(|held| {
            menu.kind() == Kind::Cartography
                || held
                    .iter()
                    .filter_map(super::maps::map_id)
                    .any(|id| self.map(id).is_some())
        })
        .map(|held| self.level_maps(&held));
        let host = self.server.lan.as_ref()?;
        // `ArmorSlot.mayPlace` asks the parts with the game as `Host`: the menu is the guest's own.
        let parts = self.menu_can_equip();
        let guest = &host.hub.guests[gi];
        self.load_guest_menu(menu, host, gi);
        let env = Env {
            maps: maps.as_ref().map(|m| m as &dyn sunrise_sim::menu::Maps),
            stacked,
            ..self.guest_slot_env(guest, &parts)
        };
        menu.brewed_potion_hook = !mod_api::__private::mods().is_empty();
        menu.item_smelted_hook = mod_api::__private::listened(mod_api::HookId::ItemSmelted);
        let result = op(menu, &env);
        if let Some(maps) = maps {
            self.register_made_maps(maps);
        }
        self.server
            .lan
            .as_mut()?
            .hub
            .guests
            .update(gi, |guest| store_player(menu, guest));
        Some(result)
    }

    pub(in crate::game) fn dispatch_item_smelted(
        &mut self,
        player: mod_api::PlayerRef,
        smelted: &[(ItemStack, u32)],
    ) {
        for (stack, amount_removed) in smelted {
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

    pub(in crate::game) fn publish_furnace_menu_effects(&mut self, effects: FurnaceMenuEffects) {
        let FurnaceMenuEffects {
            uuid,
            pos,
            at,
            award_recipes,
            smelted,
        } = effects;
        if award_recipes {
            let recipes = self.award_furnace_recipes(pos, at);
            // The extracting player learns what the furnace used, before `item_smelted`.
            let conn = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.uuid == uuid))
                .map(|g| g.conn);
            if let Some(conn) = conn {
                self.as_guest(conn, |game| game.award_recipes(&recipes));
            }
        }
        if !smelted.is_empty()
            && mod_api::__private::listened(mod_api::HookId::ItemSmelted)
            && let Some(player) = self.server_player(uuid)
        {
            self.dispatch_item_smelted(player, &smelted);
        }
    }

    /// Load the player and slot context for the menu's live queries, without storing any edits.
    fn load_guest_menu<'a>(
        &'a self,
        menu: &mut Menu,
        host: &'a super::lan::LanHost,
        gi: usize,
    ) -> Env<'a> {
        let guest = &host.hub.guests[gi];
        let open = guest.menu.as_ref();
        let limits = open
            .map(|opened| opened.slots.stack_limits(&self.common.world, opened.size))
            .unwrap_or_default();
        menu.set_container_limits(limits);
        menu.set_block_pos(
            open.filter(|opened| opened.slots != Slots::Native)
                .and_then(|opened| opened.pos)
                .map(|(x, y, z)| [x, y, z]),
        );
        load_player(menu, guest);
        Env {
            player: Some(guest.uuid),
            player_entity_id: Some(guest.entity_id),
            limited_crafting: host.hub.limited_crafting,
            known_recipes: Some(&guest.recipe_book),
            ..self.menu_env()
        }
    }

    pub(in crate::game) fn publish_guest_menu_effects(
        &mut self,
        conn: u32,
        effects: GuestMenuEffects,
    ) {
        let Some(gi) = self.guest_index(conn) else {
            return;
        };
        let guest = &self.server.lan.as_ref().unwrap().hub.guests[gi];
        let (uuid, entity_id) = (guest.uuid, guest.entity_id);
        let GuestMenuEffects {
            dropped,
            equipped,
            brewed,
            brewed_potion_takes,
            crafted,
            item_crafted,
            crafted_counts,
        } = effects;
        // `ItemStack.onCraftedBy`'s `Stats.ITEM_CRAFTED` count, before the
        // recipe the take used.
        if !crafted_counts.is_empty() {
            self.as_guest(conn, |g| {
                for (item, count) in crafted_counts {
                    g.award_stat("crafted", &item, count);
                }
            });
        }
        if !brewed.is_empty() || !brewed_potion_takes.is_empty() {
            self.as_guest(conn, |g| {
                for stack in &brewed_potion_takes {
                    g.native_player_brewed_potion(uuid, entity_id, stack);
                }
                for stack in &brewed {
                    g.trigger_brewed_potion(stack);
                }
            });
        }
        if !crafted.is_empty() {
            self.as_guest(conn, |g| {
                for (recipe, ingredients) in &crafted {
                    g.trigger_recipe_crafted(recipe, ingredients);
                }
            });
        }
        if !item_crafted.is_empty() {
            self.as_guest(conn, |g| {
                let player = g.native_acting_id();
                for (recipe, stack, inputs) in &item_crafted {
                    // ItemStack.onCraftedBy awards Stats.ITEM_CRAFTED.
                    g.award_stat("crafted", &stack.item, stack.count as i32);
                    g.native_item_crafted(player, recipe, stack, inputs);
                }
            });
        }
        if !equipped.is_empty() {
            self.as_guest(conn, |g| {
                for (slot, stack) in equipped {
                    g.player_equipped(slot, stack.as_ref());
                }
            });
        }
        self.drop_guest_items(gi, dropped);
    }

    /// `InventoryMenu.slotsChanged` for guest `uuid`'s crafting grid written outside a click
    /// (a mod's menu slot write, at the root commit): the result made again from the grid, with
    /// `native_prepare_item_craft` as a click has it, kept by the guest and their menu. The
    /// client hears it with the tick's inventory sync.
    pub(in crate::game::server_side) fn guest_craft_result(&mut self, conn: u32) {
        let Some(g) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn).filter(|g| g.placed))
        else {
            return;
        };
        let uuid = g.uuid;
        let mut result = self
            .recipes()
            .find(&self.common.loop_context, &g.craft_grid, 2)
            .map(|(_, stack)| stack);
        if mod_api::__private::listened(mod_api::HookId::PrepareItemCraft) {
            let name = g.name.clone();
            self.native_prepare_item_craft(uuid, &name, None, &mut result);
        }
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if host
            .hub
            .update_guest(conn, |g| g.craft_result.clone_from(&result))
            .is_none()
        {
            return;
        }
        let menu = host
            .menus
            .get_mut(&conn)
            .and_then(|p| p.crafting.inventory.take());
        if let Some(mut menu) = menu {
            menu.set_slot(&self.menu_env(), 0, result);
            if let Some(play) = self
                .server
                .lan
                .as_mut()
                .and_then(|host| host.menus.get_mut(&conn))
            {
                *play.crafting.inventory = Some(menu);
            }
        }
    }

    /// A click on the guest's inventory menu (window 0): `InventoryMenu`,
    /// crafting grid included. False for other windows.
    pub(in crate::game::server_side) fn lan_inventory_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        if p.container != 0 {
            return false;
        }
        let Some(host) = self.server.lan.as_mut() else {
            return true;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return true;
        };
        if host.hub.guests[gi].spectator() {
            return true;
        }
        let crafting = host.menus.entry(conn).or_default();
        let mut menu = crafting
            .crafting
            .inventory
            .take()
            .unwrap_or_else(|| Box::new(Menu::new(Kind::Inventory)));
        let grid = host.hub.guests[gi].craft_grid.clone();
        let Some(changed) = self.guest_menu_click(gi, &mut menu, p, move |menu, env| {
            for (k, s) in grid.iter().enumerate() {
                menu.set_slot(env, 1 + k, s.clone());
            }
        }) else {
            return true;
        };
        let Some(host) = self.server.lan.as_ref() else {
            return true;
        };
        let guest = &host.hub.guests[gi];
        let (uuid, name) = (guest.uuid, guest.name.clone());
        let mut result = menu.slot(0).cloned();
        self.native_prepare_item_craft(uuid, &name, None, &mut result);
        let env = self.menu_env();
        menu.set_slot(&env, 0, result);
        let Some(host) = self.server.lan.as_mut() else {
            return true;
        };
        host.hub.guests.update(gi, |g| {
            for k in 0..4 {
                g.craft_grid[k] = menu.slot(1 + k).cloned();
            }
            g.craft_result = menu.slot(0).cloned();
        });
        *host.menus.entry(conn).or_default().crafting.inventory = Some(menu);
        host.hub
            .inventory_clicked(conn, p, &self.common.world, &changed);
        true
    }

    /// Run a blockless owned menu through the same slot layout and click engine as its client.
    /// When the menu type has a `menu_slot_changed` listener, each click step's input changes
    /// reach it before the next step, and a `menu_slot_take` listener hears each take
    /// (`Slot.onTake`), in one transaction with the click (opened only then), so a failed listener
    /// undoes the click.
    pub(in crate::game::server_side) fn lan_owned_menu_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> Option<bool> {
        use super::slot_changes::{SlotChangedBacking, SlotChangedMenu, SlotSnapshots};
        use sunrise_gameplay_common::game::lan_menu_data::ClickReply;
        let host = self.server.lan.as_mut()?;
        let gi = host.hub.guests.connection_index(conn)?;
        let guest = &host.hub.guests[gi];
        let layout = guest.menu.as_ref().and_then(|m| {
            let layout = m.mod_type.filter(|t| t.owned_slots.is_some())?;
            (m.window == p.container && !guest.spectator()).then_some(layout)
        })?;
        let generation = guest.menu_generation;
        let target_uuid = guest.uuid;
        let offhand_before = guest.offhand.clone();
        let items = guest.native_menu.clone();
        let listens_take =
            mod_api::__private::menu_listens(mod_api::HookId::MenuSlotTake, layout.id);
        let slot_changed =
            mod_api::__private::menu_listens(mod_api::HookId::MenuSlotChanged, layout.id)
                .then_some(SlotChangedMenu {
                    conn,
                    uuid: target_uuid,
                    window: p.container,
                    layout,
                    backing: SlotChangedBacking::Owned { generation },
                });
        self.journal_menu_click(conn);
        let host = self.server.lan.as_mut()?;
        let mut menu = host
            .menus
            .entry(conn)
            .or_default()
            .owned_menu
            .take()
            .filter(|(g, _)| *g == generation)
            .map_or_else(|| Box::new(Menu::new(Kind::Mod(layout))), |(_, menu)| menu);
        let live = |g: &Guest| {
            g.menu_generation == generation
                && g.menu.as_ref().is_some_and(|m| m.window == p.container)
        };
        let click = super::lan_pickup::click_of(p);
        let prepare = |menu: &mut Menu, env: &Env| menu.load_container(env, &items, None);
        let snapshot = menu.snapshot();
        // A take listener edits the transient slots through the Host: the menu reads them back.
        let refresh = |game: &crate::game::ServerGame<'_>, menu: &mut Menu| {
            let Some(guest) = game
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
            else {
                return false;
            };
            if !live(guest) {
                return false;
            }
            for slot in 0..layout.slots {
                menu.load_slot(slot, guest.native_menu.get(slot).cloned().flatten());
            }
            true
        };
        // What the guest sees: a listener's writes can reach slots the click itself never touched.
        let before: Vec<_> = (0..menu.slot_count())
            .map(|s| host.hub.view_slot(gi, &self.common.world, s))
            .collect();
        let listened = slot_changed.is_some() || listens_take;
        let scope = self.storage_scope_guard(listened);
        let game = &mut *scope.game;
        if listened {
            game.transactions.last_mut().unwrap().menu_click_active = Some(conn);
            game.journal_guest_click(conn);
            game.journal_menu_click(conn);
        }
        let applied = game
            .guest_menu_action(
                gi,
                &mut menu,
                Some(click),
                Some(&prepare),
                |game, menu, stacked| {
                    let gi = game.guest_index(conn)?;
                    game.server
                        .lan
                        .as_ref()?
                        .hub
                        .guest(conn)
                        .filter(|g| live(g))?;
                    let (mut previous, rest) =
                        game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                            prepare(menu, env);
                            menu.track_takes(listens_take);
                            let before = menu.container_items();
                            (before, menu.start_click(env, click.0, click.1, click.2))
                        })?;
                    let mut rest = rest.ok()?;
                    loop {
                        let gi = game.guest_index(conn)?;
                        let after = menu.container_items();
                        game.server.lan.as_mut()?.hub.guests.update(gi, |g| {
                            g.native_menu = after.clone();
                        });
                        if let Some(target) = &slot_changed {
                            let snapshots = SlotSnapshots {
                                before: &previous,
                                after: &after,
                            };
                            if !game.mod_menu_slot_changed(target, Some(menu), snapshots) {
                                return None;
                            }
                        }
                        // `Slot.onTake`, then the inputs the take listener changed, before the
                        // click goes on: the next step sees the next result.
                        let menu_live = if let Some(take) = menu.take_notice() {
                            let before_take = menu.container_items();
                            if game.as_guest(conn, |game| {
                                game.mod_menu_slot_take(None, target_uuid, layout.id, &take)
                            }) != Some(true)
                            {
                                return None;
                            }
                            let live_now = refresh(game, menu);
                            if live_now && let Some(target) = &slot_changed {
                                let after_take = menu.container_items();
                                let snapshots = SlotSnapshots {
                                    before: &before_take,
                                    after: &after_take,
                                };
                                if !game.mod_menu_slot_changed(target, Some(menu), snapshots) {
                                    return None;
                                }
                                refresh(game, menu)
                            } else {
                                live_now
                            }
                        } else {
                            game.server
                                .lan
                                .as_ref()
                                .and_then(|host| host.hub.guest(conn))
                                .is_some_and(live)
                        };
                        let Some(step) = rest.take() else {
                            break;
                        };
                        let step = if menu_live {
                            previous = menu.container_items();
                            step
                        } else if let Some(overflow) = step.after_menu_closed() {
                            overflow
                        } else {
                            break;
                        };
                        let gi = game.guest_index(conn)?;
                        let next = game.guest_menu_edit(gi, menu, None, |menu, env| {
                            menu.resume_click(env, step)
                        })?;
                        if !menu_live {
                            break;
                        }
                        rest = next;
                    }
                    game.after_commit(After::GuestMenu(
                        conn,
                        Box::new(GuestMenuEffects::take(menu)),
                    ));
                    Some(())
                },
            )
            .is_some();
        scope.close(applied);
        if !applied {
            menu.restore(snapshot);
        }
        if let Some(host) = self.server.lan.as_mut()
            && let Some(gi) = host.hub.guests.connection_index(conn)
        {
            let guest = &host.hub.guests[gi];
            // A listener can move the off hand, which the menu's slots don't show.
            let offhand = guest.offhand != offhand_before
                && !(p.input == sunrise_sim::lan::menu::input::SWAP && p.button == 40);
            if live(guest) {
                let changed: Vec<_> = before
                    .into_iter()
                    .enumerate()
                    .filter_map(|(s, old)| {
                        (old != host.hub.view_slot(gi, &self.common.world, s)).then_some(s)
                    })
                    .collect();
                host.menus.entry(conn).or_default().owned_menu = Some((generation, menu));
                if applied {
                    self.reply_predicted_menu_click(conn, p, changed);
                } else {
                    self.reply_menu_click(conn, ClickReply::Resend(p.container));
                }
            }
            if offhand {
                self.reply_menu_click(conn, ClickReply::Offhand);
            }
        }
        Some(applied)
    }

    /// A click in a machine or block-backed mod menu. `None` means a different kind of menu.
    pub(in crate::game) fn lan_machine_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> Option<bool> {
        use sunrise_gameplay_common::game::lan_menu_data::ClickReply;
        let host = self.server.lan.as_ref()?;
        let gi = host.hub.guests.connection_index(conn)?;
        let (pos, mod_type, menu_id) = host.hub.guests[gi]
            .menu
            .as_ref()
            .filter(|m| p.container != 0 && m.window == p.container)
            .and_then(|m| match m.slots {
                Slots::Block(pos) => Some((
                    pos,
                    m.mod_type,
                    host.hub.registry_name("minecraft:menu", m.kind)?.to_owned(),
                )),
                _ => None,
            })?;
        // A mod menu type's linked slots show their containers' slots
        // (`native_menu_links`); those with nothing behind them are left out
        // of the click, as a hidden slot is.
        let linked = mod_type.and_then(|t| t.links).map(|l| {
            let links = host.hub.guests[gi]
                .menu
                .as_ref()
                .map(|m| m.links.clone())
                .unwrap_or_default();
            (l, links)
        });
        let unlinked = linked
            .as_ref()
            .map(|(l, links)| l.slots().skip(linked_slot_count(&self.common.world, links)));
        let entity = self.common.world.block_entities.get(&pos)?;
        let (kind, mut items) = machine_menu(entity, mod_type)?;
        let listens_take = matches!((kind, entity), (Kind::Mod(_), BlockEntity::Mod(_))
            if mod_api::__private::menu_listens(mod_api::HookId::MenuSlotTake, &menu_id));
        let click = super::lan_pickup::click_of(p);
        let native_quick_move = click.2 == Input::QuickMove
            && (0..=1).contains(&click.1)
            && matches!((kind, entity), (Kind::Mod(_), BlockEntity::Mod(entity))
                if mod_api::__private::block_entity_type_listens(mod_api::HookId::MenuQuickMove, entity.id()));
        let listens_changed = matches!((kind, entity), (Kind::Mod(_), BlockEntity::Mod(_))
            if mod_api::__private::menu_listens(mod_api::HookId::MenuSlotChanged, &menu_id));
        let heard = listens_take || listens_changed;
        let overlay_links = |world: &sunrise_level::world::World,
                             items: &mut [Option<ItemStack>]| {
            if let Some((l, links)) = &linked {
                LinkedSlots::new(world, *l, links).overlay(items);
            }
        };
        overlay_links(&self.common.world, &mut items);
        // CrafterBlockEntity.containerData: the disabled slots, then triggered.
        let crafter_data = match self.common.world.block_entities.get(&pos) {
            Some(BlockEntity::Crafter(c)) => Some((c.disabled, c.is_triggered())),
            _ => None,
        };
        if host.hub.guests[gi].spectator() {
            // handleContainerClick: nothing changes, and sendAllDataToRemote
            // takes back what the client predicted.
            self.reply_menu_click(conn, ClickReply::Resend(p.container));
            return Some(true);
        }
        let menu_generation = host.hub.guests[gi].menu_generation;
        let generation = self.common.world.block_entities.generation(&pos);
        let live = |guest: &Guest| {
            guest.menu_generation == menu_generation
                && guest
                    .menu
                    .as_ref()
                    .is_some_and(|open| open.window == p.container)
        };
        self.journal_menu_click(conn);
        let host = self.server.lan.as_mut()?;
        let mut menu = host
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine
            .take()
            .filter(|(window, m)| *window == p.container && m.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
        menu.set_registry_id(Some(&menu_id));
        let mut refreshed_result = None;
        if matches!(kind, Kind::Mod(_)) {
            menu.reset_mod_visibility(unlinked.into_iter().flatten());
            let host = self.server.lan.as_ref().unwrap();
            let shown = Self::lan_mod_craft_shown(host, gi, &items);
            let env = self.load_guest_menu(&mut menu, host, gi);
            // Use the shown result without assembling again. If the grid is stale,
            // prepare its new result before any rule or move reads that slot.
            let needs_preview = shown.is_none();
            menu.load_container(&env, &items, shown);
            if needs_preview && let Some(craft) = menu.mod_crafting() {
                self.lan_mod_craft_settle(gi, pos, &mut menu);
                refreshed_result = Some(craft.result);
            }
        }
        let refresh = |game: &crate::game::ServerGame<'_>, menu: &mut Menu| {
            if generation != game.world.block_entities.generation(&pos)
                || !game
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .is_some_and(live)
            {
                return false;
            }
            let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos) else {
                return false;
            };
            let mut actual = entity.items.clone();
            actual.resize(items.len(), None);
            overlay_links(&game.world, &mut actual);
            let old = menu.container_items();
            // `load_container` recomputes from the grid. Keep the current (possibly
            // customized) result when the listener only edited slots outside the grid.
            let kept_result = menu
                .mod_crafting()
                .filter(|c| c.grid_slots().all(|k| old[k] == actual[k]))
                .map(|c| menu.slot(c.result).cloned());
            let Some(host) = game.server.lan.as_ref() else {
                return false;
            };
            let Some(gi) = host.hub.guests.connection_index(conn) else {
                return false;
            };
            let env = game.load_guest_menu(menu, host, gi);
            menu.load_container(&env, &actual, kept_result);
            true
        };
        let prepare = |menu: &mut Menu, env: &Env| {
            if !matches!(kind, Kind::Mod(_)) {
                for (k, s) in items.iter().enumerate() {
                    menu.set_slot(env, k, s.clone());
                }
            }
            // After the slots: setting a disabled slot (CrafterBlockEntity.setItem) enables it.
            if let Some((disabled, triggered)) = crafter_data {
                for (k, d) in disabled.iter().enumerate() {
                    menu.data[k] = i32::from(*d);
                }
                menu.data[9] = i32::from(triggered);
            }
        };
        let pickup_checked = if native_quick_move
            && !menu.quick_crafting()
            && let Ok(slot) = usize::try_from(click.0)
            && slot < menu.slot_count()
        {
            let env = self.load_guest_menu(&mut menu, self.server.lan.as_ref().unwrap(), gi);
            if !menu.may_pickup(&env, slot) {
                // The source refused before quickMoveStack. Correct the client's prediction.
                if let Some(host) = self.server.lan.as_mut()
                    && host.hub.guest(conn).is_some_and(live)
                {
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((p.container, menu));
                    self.reply_predicted_menu_click(
                        conn,
                        p,
                        refreshed_result.into_iter().collect(),
                    );
                }
                return Some(true);
            }
            if let Some(applied) = self.lan_mod_quick_move(conn, p, Some(&menu)) {
                if !applied
                    && let Some(host) = self.server.lan.as_mut()
                    && host.hub.guest(conn).is_some_and(live)
                {
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((p.container, menu));
                }
                return Some(applied);
            }
            // Refresh the already-admitted menu without asking the pickup rule again.
            if !refresh(self, &mut menu) {
                self.reply_menu_click(conn, ClickReply::Resend(p.container));
                return Some(false);
            }
            true
        } else {
            false
        };
        // The write-back to the block's storage and its effects: inside the click's decision, so
        // an outer abort restores them with the cursor and the slots.
        let write_back =
            |game: &mut crate::game::ServerGame<'_>, menu: &mut Menu, changed: bool| {
                game.journal_block_entity(pos);
                if let Some((_, links)) = &linked {
                    for &at in links {
                        game.journal_block_entity(at);
                    }
                }
                let mut linked_after = Vec::new();
                match game.common.world.block_entities.get_mut(&pos) {
                    Some(BlockEntity::Furnace(f)) => {
                        for k in 0..3 {
                            f.items[k] = menu.slot(k).cloned();
                        }
                        // AbstractFurnaceBlockEntity.setItem: only a new input restarts cooking.
                        f.input_clicked(
                            &game.common.loop_context,
                            server_world_recipes!(game),
                            items[0].clone(),
                        );
                    }
                    // BrewingStandBlockEntity.serverTick notices a new ingredient itself.
                    Some(BlockEntity::Brewing(b)) => {
                        for k in 0..5 {
                            b.items[k] = menu.slot(k).cloned();
                        }
                    }
                    // The mod's items past its menu's slots stay as they were.
                    Some(BlockEntity::Mod(m)) => {
                        let n = items.len();
                        if m.items.len() < n {
                            m.items.resize(n, None);
                        }
                        let mut after = menu.container_items();
                        // Linked slots go back to their containers, never into the
                        // block entity.
                        if let Some((l, _)) = &linked {
                            linked_after = l
                                .slots()
                                .filter_map(|k| after.get_mut(k).map(Option::take))
                                .collect();
                        }
                        for (slot, stack) in m.items.iter_mut().zip(after) {
                            *slot = stack;
                        }
                    }
                    // CrafterBlockEntity.setItem: filling a disabled slot enables it.
                    Some(BlockEntity::Crafter(c)) => {
                        for k in 0..9 {
                            c.items[k] = menu.slot(k).cloned();
                            c.disabled[k] = menu.data[k] == 1;
                        }
                    }
                    _ => {}
                }
                // FurnaceResultSlot.checkTakeAchievements and its observable effects may run
                // only after the click, including any outer menu decision, is committed, and only for
                // actual takes (`onTake` / `onQuickCraft`), never direct `Slot.set` edits.
                let award_recipes = std::mem::take(&mut menu.furnace_awarded);
                let smelted = std::mem::take(&mut menu.item_smelted);
                if (award_recipes || !smelted.is_empty())
                    && let Some(guest) = game
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                {
                    let effects = FurnaceMenuEffects {
                        uuid: guest.uuid,
                        pos,
                        at: guest.position,
                        smelted,
                        award_recipes,
                    };
                    game.after_commit(After::FurnaceMenu(Box::new(effects)));
                }
                // Slot.setChanged on a changed container slot: BlockEntity.setChanged,
                // so the chunk is saved and comparators read the block again.
                if changed {
                    game.after_commit(After::Changed(pos));
                }
                if let Some((l, links)) = &linked {
                    let bound = linked_slot_count(&game.world, links).min(l.count);
                    linked_after.truncate(bound);
                    for at in write_links(&mut game.world, links, linked_after) {
                        game.after_commit(After::Changed(at));
                    }
                }
            };
        if heard {
            let host = self.server.lan.as_ref().unwrap();
            let before: Vec<_> = (0..menu.slot_count())
                .map(|s| host.hub.view_slot(gi, &self.common.world, s))
                .collect();
            let original = menu.snapshot();
            let uuid = host.hub.guests[gi].uuid;
            let slot_changed = mod_type.filter(|_| listens_changed).map(|layout| {
                super::slot_changes::SlotChangedMenu {
                    conn,
                    uuid,
                    window: p.container,
                    layout,
                    backing: super::slot_changes::SlotChangedBacking::Block { pos, generation },
                }
            });
            let offhand_before = host.hub.guests[gi].offhand.clone();
            let applied = self
                .storage_transaction(|game, _| {
                    // `menu_slot_changed` listeners run inside this click: they may not replace
                    // the screen speculatively, and their seen-snapshot rolls back with it.
                    game.transactions.last_mut().unwrap().menu_click_active = Some(conn);
                    game.journal_guest_click(conn);
                    game.journal_menu_click(conn);
                    game.guest_menu_action(
                        gi,
                        &mut menu,
                        Some(click),
                        Some(&prepare),
                        |game, menu, stacked| {
                            let (mut previous, mut rest) =
                                game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                                    if !pickup_checked {
                                        prepare(menu, env);
                                    }
                                    menu.track_takes(listens_take);
                                    let before = menu.container_items();
                                    let rest = if pickup_checked {
                                        menu.start_checked_quick_move(env, click.0, click.1)
                                    } else {
                                        menu.start_click(env, click.0, click.1, click.2)
                                    };
                                    (before, rest)
                                })?;
                            if rest.is_err() {
                                return None;
                            }
                            loop {
                                let after = menu.container_items();
                                write_back(game, menu, after != previous);
                                // `AbstractContainerMenu.slotsChanged` after each click step: the
                                // step's input edits, batched, before any take is heard.
                                if let Some(target) = &slot_changed {
                                    let snapshots = super::slot_changes::SlotSnapshots {
                                        before: &previous,
                                        after: &after,
                                    };
                                    if !game.mod_menu_slot_changed(target, Some(menu), snapshots) {
                                        return None;
                                    }
                                }
                                let menu_live = if let Some(take) = menu.take_notice() {
                                    let before_take = menu.container_items();
                                    if game.as_guest(conn, |game| {
                                        game.mod_menu_slot_take(Some(pos), uuid, &menu_id, &take)
                                    }) != Some(true)
                                    {
                                        return None;
                                    }
                                    let live_now = refresh(game, menu);
                                    // A take listener that consumed inputs readies the next
                                    // result before the click continues.
                                    if live_now && let Some(target) = &slot_changed {
                                        let after_take = menu.container_items();
                                        let snapshots = super::slot_changes::SlotSnapshots {
                                            before: &before_take,
                                            after: &after_take,
                                        };
                                        if !game.mod_menu_slot_changed(
                                            target,
                                            Some(menu),
                                            snapshots,
                                        ) {
                                            return None;
                                        }
                                    }
                                    live_now
                                } else if slot_changed.is_some() {
                                    // A `menu_slot_changed` listener may have replaced the block
                                    // or the menu.
                                    generation == game.world.block_entities.generation(&pos)
                                        && game
                                            .server
                                            .lan
                                            .as_ref()
                                            .and_then(|host| host.hub.guest(conn))
                                            .is_some_and(live)
                                } else {
                                    true
                                };
                                let Some(step) = rest.ok()? else {
                                    break;
                                };
                                let step = if menu_live {
                                    // Listener edits are now the step's inputs. Don't overwrite them.
                                    previous = menu.container_items();
                                    step
                                } else {
                                    let Some(owed) = step.after_menu_closed() else {
                                        break;
                                    };
                                    // Losing the block queues a close, so the admitted cursor is
                                    // still live here. Never settle this write on a different window.
                                    game.server
                                        .lan
                                        .as_ref()?
                                        .hub
                                        .guest(conn)
                                        .filter(|g| live(g))?;
                                    owed
                                };
                                let next =
                                    game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                                        menu.resume_click(env, step)
                                    })?;
                                if !menu_live {
                                    // Only the player-side write was owed. Don't write back the menu.
                                    break;
                                }
                                rest = Ok(next);
                            }
                            menu.track_takes(false);
                            game.after_commit(After::GuestMenu(
                                conn,
                                Box::new(GuestMenuEffects::take(menu)),
                            ));
                            Some(())
                        },
                    )
                    .ok_or(())
                })
                .is_ok();
            if applied && refresh(self, &mut menu) {
                self.lan_mod_craft_settle(gi, pos, &mut menu);
            }
            if !applied {
                menu.restore(original);
            }
            if let Some(host) = self.server.lan.as_mut()
                && let Some(guest) = host.hub.guest(conn)
            {
                let offhand = guest.offhand != offhand_before
                    && !(p.input == sunrise_sim::lan::menu::input::SWAP && p.button == 40);
                if live(guest) {
                    let reply = if !applied {
                        ClickReply::Resend(p.container)
                    } else if generation != self.common.world.block_entities.generation(&pos) {
                        ClickReply::Close(p.container)
                    } else {
                        let changed = before
                            .into_iter()
                            .enumerate()
                            .filter_map(|(s, old)| {
                                (refreshed_result == Some(s)
                                    || old != host.hub.view_slot(gi, &self.common.world, s))
                                .then_some(s)
                            })
                            .collect();
                        ClickReply::Predicted(p.clone(), changed)
                    };
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((p.container, menu));
                    self.reply_menu_click(conn, reply);
                }
                if offhand {
                    self.reply_menu_click(conn, ClickReply::Offhand);
                }
            }
            return Some(applied);
        }
        let original = menu.snapshot();
        let changed = self.guest_menu_action(
            gi,
            &mut menu,
            Some(click),
            Some(&prepare),
            |game, menu, stacked| {
                let changed: Vec<_> = game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                    if !pickup_checked {
                        prepare(menu, env);
                    }
                    let before: Vec<_> = (0..menu.slot_count())
                        .map(|s| menu.slot(s).cloned())
                        .collect();
                    if pickup_checked {
                        menu.start_checked_quick_move(env, click.0, click.1).ok()?;
                    } else {
                        menu.click(env, click.0, click.1, click.2).ok()?;
                    }
                    Some(
                        (0..menu.slot_count())
                            .filter(|&s| menu.slot(s) != before[s].as_ref())
                            .collect(),
                    )
                })??;
                write_back(game, menu, changed.iter().any(|&s| s < items.len()));
                game.after_commit(After::GuestMenu(
                    conn,
                    Box::new(GuestMenuEffects::take(menu)),
                ));
                Some(changed)
            },
        );
        let applied = changed.is_some();
        let changed = if let Some(mut changed) = changed {
            if self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(live)
            {
                for slot in self
                    .lan_mod_craft_settle(gi, pos, &mut menu)
                    .into_iter()
                    .chain(refreshed_result)
                {
                    if !changed.contains(&slot) {
                        changed.push(slot);
                    }
                }
            }
            Some(changed)
        } else {
            menu.restore(original);
            None
        };
        if let Some(host) = self.server.lan.as_mut()
            && host.hub.guest(conn).is_some_and(live)
        {
            *host.menus.entry(conn).or_default().crafting.machine = Some((p.container, menu));
            if let Some(changed) = changed {
                self.reply_predicted_menu_click(conn, p, changed);
            } else {
                self.reply_menu_click(conn, ClickReply::Resend(p.container));
            }
        }
        Some(applied)
    }

    /// `ServerGamePacketListenerImpl.handleContainerSlotStateChanged`: a
    /// guest's crafter slot toggle (`CrafterBlockEntity.setSlotState`) in
    /// their open crafter window.
    pub(in crate::game::server_side) fn lan_crafter_slot_state(
        &mut self,
        conn: u32,
        window: i32,
        slot: i32,
        enabled: bool,
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(pos) = host
            .hub
            .guest(conn)
            .filter(|g| !g.spectator())
            .and_then(|g| g.menu.as_ref())
            .filter(|m| window != 0 && m.window == window)
            .and_then(|m| match m.slots {
                Slots::Block(pos) => Some(pos),
                _ => None,
            })
        else {
            return;
        };
        if let (Some(BlockEntity::Crafter(c)), Ok(slot)) = (
            self.common.world.block_entities.get_mut(&pos),
            usize::try_from(slot),
        ) {
            c.set_slot_state(slot, enabled);
            self.common.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        }
    }

    /// ServerGamePacketListenerImpl.handlePlaceRecipe: the guest's recipe
    /// book places a known recipe in their inventory grid, crafting table or furnace
    /// (`ServerPlaceRecipe`), and answers with a ghost recipe when the items
    /// are missing.
    pub(in crate::game) fn lan_place_recipe(
        &mut self,
        conn: u32,
        p: &sunrise_protocol::packets::recipes::PlaceRecipe,
    ) {
        self.lan_route_place_recipe(
            conn,
            RecipePlacementIntent::Vanilla {
                container: p.container,
                display: p.recipe,
                use_max: p.use_max,
            },
        );
    }

    pub(in crate::game) fn lan_route_place_recipe(
        &mut self,
        conn: u32,
        intent: RecipePlacementIntent<'_>,
    ) {
        let Some((player, name, container, id, use_max, generation, dimension)) = (|| {
            let host = self.server.lan.as_ref()?;
            let gi = host.hub.guests.iter().position(|g| g.conn == conn)?;
            let g = &host.hub.guests[gi];
            if !g.placed || !host.hub.here(g) || g.dead || g.health <= 0.0 || g.spectator() {
                return None;
            }
            let (container, use_max) = match intent {
                RecipePlacementIntent::Vanilla {
                    container, use_max, ..
                } => (container, use_max),
                RecipePlacementIntent::ModBook {
                    catalog, use_max, ..
                } => (catalog.window, use_max),
            };
            if container != guest_open_window(host, conn) {
                return None;
            }
            // The server menu uses the world's active data-pack tables, the same
            // tables used by the guest menu below. The base resources can have a
            // different display registry or placement for an overridden recipe.
            let id = match intent {
                RecipePlacementIntent::Vanilla { display, .. } => server_world_stations!(self)
                    .recipe_index
                    .recipe_of_display(display)?
                    .to_owned(),
                RecipePlacementIntent::ModBook { display, .. } => {
                    display.recipe.as_str().to_owned()
                }
            };
            // An unknown or impossible recipe is refused outright.
            if !g.recipe_book.knows(&id) {
                return None;
            }
            if matches!(intent, RecipePlacementIntent::Vanilla { .. })
                && server_world_recipes!(self)
                    .placement(&id)
                    .is_none_or(|placement| placement.impossible())
            {
                return None;
            }
            Some((
                g.uuid,
                g.name.clone(),
                container,
                id,
                use_max,
                g.menu_generation,
                g.dimension.clone(),
            ))
        })() else {
            if let RecipePlacementIntent::ModBook { display, .. } = intent {
                log::warn!(
                    "[recipe book] refused mod-book placement for {} on conn {conn}: unavailable or unknown",
                    display.recipe.as_str()
                );
            }
            return;
        };
        if !self.guest_menu_still_valid(conn) {
            if let RecipePlacementIntent::ModBook { display, .. } = intent {
                log::warn!(
                    "[recipe book] refused mod-book placement for {} on conn {conn}: invalid menu",
                    display.recipe.as_str()
                );
            }
            return;
        }
        match intent {
            RecipePlacementIntent::ModBook {
                catalog,
                display,
                use_max,
            } => match self.server_mod_book_place(conn, catalog, display, use_max) {
                Ok(PostPlace::Nothing) => {}
                Ok(PostPlace::Ghost) => {
                    // A policy or callback may have closed or replaced the menu.
                    if self.mod_book_opening_matches(
                        conn,
                        catalog.window,
                        catalog.instance,
                        catalog.menu.as_str(),
                    ) {
                        match self.server_mod_book_display(catalog, display) {
                            Ok(resolved) => self.send_guest_mod_book_placement(
                                conn,
                                sunrise_sim::recipe_book::placement_sync::Frame::Ghost {
                                    catalog: catalog.clone(),
                                    recipe: display.recipe.clone(),
                                    display: resolved,
                                },
                            ),
                            Err(reason) => log::warn!(
                                "[recipe book] skipped mod-book ghost for {} on conn {conn}: {reason:?}",
                                display.recipe.as_str()
                            ),
                        }
                    }
                }
                Err(reason) => {
                    log::warn!(
                        "[recipe book] refused mod-book placement for {} on conn {conn}: {reason:?}",
                        display.recipe.as_str()
                    );
                }
            },
            RecipePlacementIntent::Vanilla { display, .. } => {
                self.native_player_recipe_book_click(player, &name, &id, use_max);
                // The writable listener may have replaced the menu, moved the player or changed
                // their mode: place only into the menu the click named, with the live mode.
                let Some((gi, creative)) =
                    self.recipe_click_menu_still_current(conn, container, generation, &dimension)
                else {
                    return;
                };
                let placed = if container == 0
                    && mod_api::__private::menu_listens(
                        mod_api::HookId::RecipeBookPlace,
                        sunrise_sim::menu::INVENTORY_MENU,
                    ) {
                    match self.inventory_book_place(player, &id, use_max, creative) {
                        mod_api::RecipeBookPlacement::Nothing => None,
                        mod_api::RecipeBookPlacement::Ghost => Some(PostPlace::Ghost),
                        mod_api::RecipeBookPlacement::Grid { .. } => {
                            // The Part ran with the player swapped in: place only into the menu
                            // the click named, with the mode it left.
                            let Some((gi, creative)) = self.recipe_click_menu_still_current(
                                conn, container, generation, &dimension,
                            ) else {
                                return;
                            };
                            self.lan_place_in_current_menu(gi, container, &id, use_max, creative)
                        }
                    }
                } else {
                    self.lan_place_in_current_menu(gi, container, &id, use_max, creative)
                };
                if placed == Some(PostPlace::Ghost)
                    && self
                        .recipe_click_menu_still_current(conn, container, generation, &dimension)
                        .is_some()
                    && let Some(host) = self.server.lan.as_mut()
                {
                    host.hub.send_ghost_recipe(conn, container, display);
                }
            }
        }
    }

    fn lan_place_in_current_menu(
        &mut self,
        gi: usize,
        container: i32,
        id: &str,
        use_max: bool,
        creative: bool,
    ) -> Option<PostPlace> {
        let conn = self.server.lan.as_ref()?.hub.guests.get(gi)?.conn;
        // AbstractFurnaceMenu.handlePlacement: the current furnace's slots,
        // rather than the separate crafting-table menu, own cooking inputs.
        let furnace = self
            .server
            .lan
            .as_ref()?
            .hub
            .guests
            .get(gi)?
            .menu
            .as_ref()
            .filter(|m| container != 0 && m.window == container)
            .and_then(|m| match m.slots {
                Slots::Block(pos) => Some(pos),
                _ => None,
            })
            .and_then(|pos| match self.common.world.block_entities.get(&pos) {
                Some(BlockEntity::Furnace(f)) => {
                    let recipe = match f.kind.recipe_type() {
                        "minecraft:blasting" => "blasting",
                        "minecraft:smoking" => "smoking",
                        _ => "smelting",
                    };
                    Some((pos, Kind::Furnace(recipe), f.items.clone()))
                }
                _ => None,
            });
        if let Some((pos, kind, items)) = furnace {
            // Opening need not have created the click/drag cache yet.
            let mut menu = self
                .server
                .lan
                .as_mut()?
                .menus
                .entry(conn)
                .or_default()
                .crafting
                .machine
                .take()
                .filter(|(window, m)| *window == container && m.kind() == kind)
                .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
            let placed = self.guest_menu_op(gi, &mut menu, |menu, env| {
                for (k, item) in items.iter().enumerate() {
                    menu.set_slot(env, k, item.clone());
                }
                menu.place_recipe(env, id, use_max, creative, None)
            });
            let changed = (0..3).any(|k| menu.slot(k) != items[k].as_ref());
            if let Some(BlockEntity::Furnace(f)) = self.common.world.block_entities.get_mut(&pos) {
                for k in 0..3 {
                    f.items[k] = menu.slot(k).cloned();
                }
                f.input_clicked(
                    &self.common.loop_context,
                    server_world_recipes!(self),
                    items[0].clone(),
                );
            }
            if changed {
                self.container_changed(pos);
            }
            if let Some(host) = self.server.lan.as_mut() {
                *host.menus.entry(conn).or_default().crafting.machine = Some((container, menu));
            }
            // Hub::sync_containers broadcasts the actual changed slots.
            placed
        } else {
            // Handles inventory (window 0), crafting table, and mod crafting menus.
            self.lan_apply_recipe(gi, container, |menu, env| {
                menu.place_recipe(env, id, use_max, creative, None)
            })
        }
    }

    /// The guest's index and live creative mode if their open menu is still the instance
    /// (`generation`) a recipe click named in `container`, in the level it was made in, and the
    /// player may still place recipes.
    fn recipe_click_menu_still_current(
        &self,
        conn: u32,
        container: i32,
        generation: u64,
        dimension: &str,
    ) -> Option<(usize, bool)> {
        let host = self.server.lan.as_ref()?;
        let gi = host.hub.guests.iter().position(|g| g.conn == conn)?;
        let g = &host.hub.guests[gi];
        (g.placed
            && host.hub.here(g)
            && !g.dead
            && g.health > 0.0
            && !g.spectator()
            && g.menu_generation == generation
            && g.dimension == dimension
            && container == guest_open_window(host, conn))
        .then(|| (gi, g.creative()))
    }

    /// `menu_transfer` for the guest's open mod menu in window `container`: the part of its
    /// type moves `recipe`'s ingredients itself (or, with `dry_run`, only says what that would
    /// do and which cells are missing or craftable, `menu_transfer_answer`). None when no part
    /// answers, the window isn't the guest's mod menu, or no mod listens.
    fn guest_menu_transfer(
        &mut self,
        gi: usize,
        container: i32,
        recipe: &str,
        max: bool,
        dry_run: bool,
    ) -> Option<mod_api::RecipeTransferCheck> {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuTransfer) {
            return None;
        }
        let g = &self.server.lan.as_ref()?.hub.guests[gi];
        let menu = g
            .menu
            .as_ref()
            .filter(|menu| menu.window == container && menu.mod_type.is_some())?;
        if !mod_api::__private::menu_listens(mod_api::HookId::MenuTransfer, &menu.registry_id) {
            return None;
        }
        let (player, menu_id) = (g.uuid, menu.registry_id.to_string());
        let pos = menu.pos.map(|(x, y, z)| [x, y, z]);
        self.menu_transfer_answer(player, &menu_id, pos, recipe, max, dry_run)
    }

    pub fn guest_recipe_transfer(&mut self, conn: u32, data: &[u8]) {
        use sunrise_core::vanilla_bytes::Writer;
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        if g.brand != "sunrise" || g.spectator() {
            return;
        }
        let Ok((container, id, all)) = sunrise_sim::lan::recipes::decode_transfer_request(data)
        else {
            return;
        };
        let blocked = container == 0 && (g.menu.is_some() || g.creative());
        // A mod menu's part moves the ingredients itself; without one the grid transfer runs.
        let answered = self.guest_menu_transfer(gi, container, &id, all, false);
        let result = if let Some(answer) = answered {
            recipes::transfer_outcome(answer.result)
        } else if blocked {
            Err(TransferError::Unavailable)
        } else {
            self.lan_apply_recipe(gi, container, |menu, env| {
                menu.transfer_recipe(env, &id, all)
            })
            .unwrap_or(Err(TransferError::Unavailable))
        };
        let mut reply = Writer::new();
        reply.varint(container);
        reply.u8(recipes::result_byte(recipes::transfer_result(result)));
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.guests[gi].send(&C::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: recipes::TRANSFER_CHANNEL.into(),
                    data: reply.data,
                },
            ));
        }
    }

    /// A viewer's check of a transfer (`sunrise:recipe_check`): the matching
    /// and placement `guest_recipe_transfer` makes, the mods' recipe sources
    /// counted, on a copy of the guest's menu (`lan_check_recipe`). Nothing is
    /// written back: no slot, block entity or `container_changed`, and no
    /// packet but the reply. Each guest can burst ten checks, then gains one
    /// check per server tick (twenty per second at 20 TPS); more go unanswered.
    pub fn guest_recipe_check(&mut self, conn: u32, data: &[u8]) {
        use sunrise_protocol::buf::Reader;
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        if g.brand != "sunrise" || g.spectator() {
            return;
        }
        let unavailable = g.menu.is_some() || g.creative();
        let mut reader = Reader::new(data);
        let (Ok(container), Ok(tag), Ok(id)) =
            (reader.varint(), reader.varint(), reader.string(512))
        else {
            return;
        };
        if reader.remaining() != 0 {
            return;
        }
        let play = host.play.entry(conn).or_default();
        if !play.recipe_check_available() {
            return;
        }
        play.recipe_checks += RECIPE_CHECK_COST;
        // A mod menu's part answers with its own cells; the grid's check names the
        // missing ones with `Missing` and never craftable ones.
        let answered = self.guest_menu_transfer(gi, container, &id, false, true);
        let check = answered.unwrap_or_else(|| {
            let check = if container == 0 && unavailable {
                None
            } else {
                self.lan_check_recipe(gi, container, &id)
            }
            .unwrap_or(TransferCheck {
                result: Err(TransferError::Unavailable),
                missing: Vec::new(),
            });
            let missing = check.result == Err(TransferError::Missing);
            mod_api::RecipeTransferCheck {
                result: recipes::transfer_result(check.result),
                missing_cells: (check.missing.iter())
                    .filter(|_| missing)
                    .filter_map(|&c| u32::try_from(c).ok())
                    .collect(),
                craftable_cells: Vec::new(),
            }
        });
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.guests[gi].send(&C::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: recipes::CHECK_CHANNEL.into(),
                    data: recipes::encode_check_reply(container, tag, &check),
                },
            ));
        }
    }

    /// `Menu::check_transfer` of recipe `id` on a copy of guest `gi`'s window
    /// `container`, built as `lan_apply_recipe` builds it: the inventory's
    /// grid, a crafting table's, or a mod crafting menu's over its block
    /// entity with its recipe sources. None when that window is none of them
    /// (the window must be the one the guest has open, as for a transfer).
    pub fn lan_check_recipe(
        &mut self,
        gi: usize,
        container: i32,
        id: &str,
    ) -> Option<TransferCheck> {
        let conn = self.server.lan.as_ref()?.hub.guests[gi].conn;
        let menu = if let Some((pos, t)) = self.lan_mod_crafting_window(gi, container) {
            let Some(BlockEntity::Mod(m)) = self.common.world.block_entities.get(&pos) else {
                return None;
            };
            let mut items = m.items.clone();
            items.resize(t.slots, None);
            let sources = self.lan_recipe_sources(gi, t, pos);
            let host = self.server.lan.as_ref()?;
            let shown = Self::lan_mod_craft_shown(host, gi, &items);
            let limits = (host.hub.guests[gi].menu.as_ref())
                .map(|opened| opened.slots.stack_limits(&self.common.world, opened.size))
                .unwrap_or_default();
            let env = self.menu_env();
            let mut menu = Menu::new(Kind::Mod(t));
            menu.set_container_limits(limits);
            menu.sources = sources.into_iter().map(|(_, items)| items).collect();
            load_player(&mut menu, &host.hub.guests[gi]);
            menu.load_container(&env, &items, shown);
            menu
        } else if container == 0 {
            let host = self.server.lan.as_ref()?;
            let env = self.menu_env();
            let mut menu = Menu::new(Kind::Inventory);
            load_player(&mut menu, &host.hub.guests[gi]);
            for (k, s) in host.hub.guests[gi].craft_grid.iter().enumerate() {
                menu.set_slot(&env, 1 + k, s.clone());
            }
            menu
        } else {
            let host = self.server.lan.as_ref()?;
            let (_, _, table, _) = (host.menus.get(&conn)?.crafting.table.as_ref())
                .filter(|(window, ..)| *window == container)?;
            let mut menu = Menu::clone(table);
            load_player(&mut menu, &host.hub.guests[gi]);
            menu
        };
        let host = self.server.lan.as_ref()?;
        let env = Env {
            player: Some(host.hub.guests[gi].uuid),
            player_entity_id: Some(host.hub.guests[gi].entity_id),
            loop_context: &self.common.loop_context,
            limited_crafting: host.hub.limited_crafting,
            can_equip: None,
            known_recipes: Some(&host.hub.guests[gi].recipe_book),
            ..self.menu_env()
        };
        Some(menu.check_transfer(&env, id))
    }

    pub fn lan_apply_recipe<R>(
        &mut self,
        gi: usize,
        container: i32,
        apply: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        if self.lan_mod_crafting_window(gi, container).is_some() {
            return self.lan_mod_craft_apply(gi, container, apply);
        }
        let conn = self.server.lan.as_ref()?.hub.guests[gi].conn;
        let host = self.server.lan.as_mut()?;
        let placed = if container == 0 {
            let crafting = host.menus.entry(conn).or_default();
            let mut menu = crafting
                .crafting
                .inventory
                .take()
                .unwrap_or_else(|| Box::new(Menu::new(Kind::Inventory)));
            let grid = host.hub.guests[gi].craft_grid.clone();
            let placed = self.guest_menu_op(gi, &mut menu, |menu, env| {
                for (k, s) in grid.into_iter().enumerate() {
                    menu.set_slot(env, 1 + k, s);
                }
                apply(menu, env)
            })?;
            let host = self.server.lan.as_mut()?;
            host.hub.guests.update(gi, |g| {
                for k in 0..4 {
                    g.craft_grid[k] = menu.slot(1 + k).cloned();
                }
                g.craft_result = menu.slot(0).cloned();
            });
            *host.menus.entry(conn).or_default().crafting.inventory = Some(menu);
            host.hub.send_inventory(gi);
            placed
        } else {
            let (window, state, mut menu, pos) = host
                .menus
                .get_mut(&conn)
                .and_then(|play| play.crafting.table.take_if(|(w, ..)| *w == container))?;
            let placed = self.guest_menu_op(gi, &mut menu, apply)?;
            let host = self.server.lan.as_mut()?;
            *host.menus.entry(conn).or_default().crafting.table = Some((window, state, menu, pos));
            self.lan_send_table_content(conn);
            placed
        };
        Some(placed)
    }

    /// The guest opened a crafting table (`CraftingTableBlock.getMenuProvider`).
    pub fn lan_open_crafting(&mut self, conn: u32, pos: [i32; 3]) {
        if self.guest_may_open(conn, "minecraft:crafting", Some(pos)) != Some(true) {
            return;
        }
        self.guest_replace_menu(conn, false, |game| {
            let Some(host) = game.server.lan.as_mut() else {
                return;
            };
            let title = sunrise_protocol::text::translatable("container.crafting", &[]);
            let Some(window) = host.hub.open_window(conn, "minecraft:crafting", title) else {
                return;
            };
            let mut menu = Box::new(Menu::new(Kind::Crafting));
            menu.set_registry_id(Some("minecraft:crafting"));
            *host.menus.entry(conn).or_default().crafting.table = Some((window, 0, menu, pos));
            game.lan_send_table_content(conn);
            game.player_menu_opening(conn);
        })
    }

    /// A click on the guest's crafting table window; false for other windows.
    pub(in crate::game::server_side) fn lan_crafting_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        let Some(play) = host.menus.get_mut(&conn) else {
            return false;
        };
        let Some((window, mut state, mut menu, pos)) = play
            .crafting
            .table
            .take_if(|(window, ..)| *window == p.container)
        else {
            return false;
        };
        let owner = play.crafting.table.stamp();
        // A full snapshot on the current state would undo clicks the client has since predicted.
        let mut full = true;
        // A table that is no longer valid takes no clicks (handleContainerClick).
        let gi = host.hub.guests.iter().position(|g| g.conn == conn);
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        if let Some(gi) = gi
            && !host.hub.guests[gi].spectator()
            && self.lan_table_valid(&host.hub.guests[gi], pos)
        {
            let click = super::lan_pickup::click_of(p);
            let Some(mut changed) =
                self.guest_menu_op_decided(gi, &mut menu, Some(click), None, |menu, env| {
                    let before: Vec<_> = (0..menu.slot_count())
                        .map(|s| menu.slot(s).cloned())
                        .collect();
                    let _ = menu.click(env, click.0, click.1, click.2);
                    (0..menu.slot_count())
                        .filter(|&s| menu.slot(s) != before[s].as_ref())
                        .collect::<Vec<_>>()
                })
            else {
                // A refused click keeps its menu open. Don't put it back if the
                // listener closed or replaced the owner while the table was lifted.
                if let Some(play) = self
                    .server
                    .lan
                    .as_mut()
                    .and_then(|host| host.menus.get_mut(&conn))
                    && play.crafting.table.stamp() == owner
                {
                    play.crafting
                        .table
                        .restore(Some((window, state, menu, pos)));
                    self.lan_send_table_content(conn);
                }
                return true;
            };
            let Some(guest) = self.server.lan.as_ref().map(|host| &host.hub.guests[gi]) else {
                return true;
            };
            let (uuid, name) = (guest.uuid, guest.name.clone());
            let mut result = menu.slot(0).cloned();
            let vanilla = result.clone();
            self.native_prepare_item_craft(uuid, &name, Some(pos), &mut result);
            if result != vanilla && !changed.contains(&0) {
                changed.push(0);
            }
            let env = self.menu_env();
            menu.set_slot(&env, 0, result);
            let Some(host) = self.server.lan.as_mut() else {
                return true;
            };
            full = p.state != state;
            if !full {
                let slots: Vec<_> = (0..menu.slot_count())
                    .map(|s| menu.slot(s).cloned())
                    .collect();
                let corrected = host
                    .hub
                    .sync_click_prediction(gi, p, &mut state, &slots, |slot| {
                        changed.contains(&slot)
                    });
                // The inventory menu's broadcast shouldn't repeat what the click settled.
                host.hub.guests.update(gi, |g| {
                    let settled = p
                        .changed
                        .iter()
                        .map(|(s, _)| *s as usize)
                        .chain(changed)
                        .chain(corrected);
                    for s in settled.filter(|s| (10..slots.len()).contains(s)) {
                        g.menu_sent[s - 1] = slots[s].clone();
                    }
                });
            }
        }
        let Some(host) = self.server.lan.as_mut() else {
            return true;
        };
        *host.menus.entry(conn).or_default().crafting.table = Some((window, state, menu, pos));
        if full {
            self.lan_send_table_content(conn);
        }
        true
    }

    /// `CraftingMenu.stillValid`: the block is still a crafting table and the
    /// guest is within reach of it.
    pub(in crate::game) fn lan_table_valid(&self, g: &Guest, pos: [i32; 3]) -> bool {
        let name = &self
            .res
            .facts()
            .get(self.common.world.block(pos[0], pos[1], pos[2]))
            .name;
        &**name == "minecraft:crafting_table"
            && g.within_interaction_range(&self.common.loop_context, pos, 4.0)
    }

    /// Guest `gi`'s open window `container` when it is a mod menu type with a
    /// crafting grid: its block and its type.
    fn lan_mod_crafting_window(
        &self,
        gi: usize,
        container: i32,
    ) -> Option<((i32, i32, i32), &'static ModMenuType)> {
        let host = self.server.lan.as_ref()?;
        let m = host.hub.guests[gi]
            .menu
            .as_ref()
            .filter(|m| container != 0 && m.window == container)?;
        let Slots::Block(pos) = m.slots else {
            return None;
        };
        let t = m.mod_type.filter(|t| t.crafting.is_some())?;
        matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Mod(_))
        )
        .then_some((pos, t))
    }

    /// `apply` (a recipe book placement or a viewer's transfer) on guest
    /// `gi`'s open mod crafting menu `container`, over its block entity's
    /// slots as a click is; None when that window is no such menu. Changes
    /// reach the guest with the tick's container sync.
    fn lan_mod_craft_apply<R>(
        &mut self,
        gi: usize,
        container: i32,
        apply: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        let (pos, t) = self.lan_mod_crafting_window(gi, container)?;
        let Some(BlockEntity::Mod(m)) = self.common.world.block_entities.get(&pos) else {
            return None;
        };
        let mut items = m.items.clone();
        items.resize(t.slots, None);
        let host = self.server.lan.as_mut()?;
        let conn = host.hub.guests[gi].conn;
        let kind = Kind::Mod(t);
        let mut menu = host
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine
            .take()
            .filter(|(window, m)| *window == container && m.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
        let shown = Self::lan_mod_craft_shown(self.server.lan.as_ref()?, gi, &items);
        let sources = self.lan_recipe_sources(gi, t, pos);
        menu.sources = sources.iter().map(|(_, items)| items.clone()).collect();
        let out = self.guest_menu_op(gi, &mut menu, |menu, env| {
            menu.load_container(env, &items, shown);
            apply(menu, env)
        })?;
        for ((at, before), after) in sources.into_iter().zip(std::mem::take(&mut menu.sources)) {
            if after == before {
                continue;
            }
            match self.common.world.block_entities.get_mut(&at) {
                Some(BlockEntity::Chest(items)) => *items = after,
                Some(BlockEntity::Mod(m)) => {
                    for (slot, stack) in m.items.iter_mut().zip(after) {
                        *slot = stack;
                    }
                }
                _ => {}
            }
            self.container_changed(at);
        }
        let after = menu.container_items();
        if after != items {
            if let Some(BlockEntity::Mod(m)) = self.common.world.block_entities.get_mut(&pos) {
                if m.items.len() < after.len() {
                    m.items.resize(after.len(), None);
                }
                for (slot, stack) in m.items.iter_mut().zip(after) {
                    *slot = stack;
                }
            }
            self.container_changed(pos);
        }
        self.lan_mod_craft_settle(gi, pos, &mut menu);
        *self
            .server
            .lan
            .as_mut()?
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine = Some((container, menu));
        Some(out)
    }

    /// The containers recipe placement into guest `gi`'s mod crafting menu
    /// `t` at `pos` may also take from (`native_recipe_sources`): each block
    /// and its slots, chest-like or a mod's container, none twice, not `pos`.
    pub fn lan_recipe_sources(
        &mut self,
        gi: usize,
        t: &ModMenuType,
        pos: (i32, i32, i32),
    ) -> Vec<((i32, i32, i32), Vec<Option<ItemStack>>)> {
        let mut asked = Vec::new();
        let Some(uuid) = self
            .server
            .lan
            .as_ref()
            .map(|host| host.hub.guests[gi].uuid)
        else {
            return Vec::new();
        };
        self.native_recipe_sources(uuid, t.id, [pos.0, pos.1, pos.2], &mut asked);
        let mut sources = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for [x, y, z] in asked {
            let at = (x, y, z);
            if at == pos || !seen.insert(at) {
                continue;
            }
            let Some(items) = self.linkable_container(at) else {
                continue;
            };
            sources.push((at, items.to_vec()));
        }
        sources
    }

    /// The slots of the block container at `at` that a mod may lend a menu
    /// (`native_recipe_sources`, `native_menu_links`): a chest-like block
    /// entity's (chests, barrels, shulker boxes, hoppers, dispensers,
    /// droppers) or a mod container's. None for anything else.
    fn linkable_container(&self, at: (i32, i32, i32)) -> Option<&[Option<ItemStack>]> {
        let state = self.common.world.block(at.0, at.1, at.2).0;
        match self.common.world.block_entities.get(&at)? {
            BlockEntity::Chest(items) => Some(items.as_slice()),
            BlockEntity::Mod(m) if !m.id().is_empty() => {
                let c = self.common.world.block_entities.mod_container(state)?;
                Some(&m.items[..c.slots.min(m.items.len())])
            }
            _ => None,
        }
    }

    /// One guest's open mod menu with linked slots (`links`): the mods name the containers
    /// behind them again (`native_menu_links`), before the hub sends the tick's slot changes.
    pub(in crate::game) fn lan_menu_links_tick_one(&mut self, conn: u32) {
        if !mod_api::__private::listened(mod_api::HookId::MenuLinks) {
            return;
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return;
        };
        self.lan_menu_links_tick_at(gi);
    }

    fn lan_menu_links_tick_at(&mut self, gi: usize) {
        let host = self.server.lan.as_ref().unwrap();
        let g = &host.hub.guests[gi];
        let Some((t, pos)) = g
            .menu
            .as_ref()
            .filter(|_| host.hub.here(g) && !self.player_tick_canceled(g.conn))
            .and_then(|m| match (m.mod_type, m.slots) {
                (Some(t), Slots::Block(pos)) if t.links.is_some() => Some((t, pos)),
                _ => None,
            })
        else {
            return;
        };
        let (conn, uuid, window, generation) = (
            g.conn,
            g.uuid,
            g.menu.as_ref().map_or(0, |m| m.window),
            g.menu_generation,
        );
        let mut scratch = std::mem::take(&mut self.server.link_scratch);
        scratch.asked.clear();
        self.native_menu_links(uuid, t.id, [pos.0, pos.1, pos.2], &mut scratch.asked);
        scratch.links.clear();
        scratch.seen.clear();
        for &at in &scratch.asked {
            let at = (at[0], at[1], at[2]);
            if at == pos || !scratch.seen.insert(at) {
                continue;
            }
            if self.linkable_container(at).is_some() {
                scratch.links.push(at);
            }
        }
        // A writable listener may have replaced the menu while the answers were collected:
        // they belong to the original level's menu instance only.
        let still = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
            .filter(|&gi| {
                let g = &self.server.lan.as_ref().unwrap().hub.guests[gi];
                g.menu_generation == generation
                    && g.menu.as_ref().is_some_and(|m| m.window == window)
            });
        if let Some(gi) = still {
            self.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(gi, |g| {
                    if let Some(m) = g.menu.as_mut() {
                        m.links.clear();
                        m.links.extend(scratch.links.iter().copied());
                    }
                });
        }
        self.server.link_scratch = scratch;
    }

    /// The guests' open mod menus with linked slots (`links`): the mods name
    /// the containers behind them again (`native_menu_links`), before the
    /// hub sends the tick's slot changes, so a tab the player picked or a
    /// chest placed or broken shows in the same tick.
    #[cfg(any(test, feature = "test-support"))]
    pub fn lan_menu_links_tick(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::MenuLinks) {
            return;
        }
        let Some(count) = self.server.lan.as_ref().map(|host| host.hub.guests.len()) else {
            return;
        };
        for gi in 0..count {
            self.lan_menu_links_tick_at(gi);
        }
    }

    /// The result guest `gi`'s mod crafting menu shows (`Some(None)` if empty),
    /// while the block entity's grid (`items`) is the one it was made from.
    fn lan_mod_craft_shown(
        host: &super::lan::LanHost,
        gi: usize,
        items: &[Option<ItemStack>],
    ) -> Option<Option<ItemStack>> {
        let menu = host.hub.guests[gi].menu.as_ref()?;
        let c = menu.mod_type?.crafting?;
        let craft = menu.craft.as_ref()?;
        (items.get(c.grid_slots()) == Some(&craft.grid[..])).then(|| craft.result.clone())
    }

    /// After the menu engine ran on guest `gi`'s mod crafting menu at `pos`:
    /// the mods have their say on the result (`native_prepare_item_craft`,
    /// as on a crafting table's), which goes into `menu` and is what the guest
    /// is shown from now on. The result's slot when the mods changed it.
    fn lan_mod_craft_settle(
        &mut self,
        gi: usize,
        pos: (i32, i32, i32),
        menu: &mut Menu,
    ) -> Option<usize> {
        self.lan_mod_craft_settle_at(gi, Some(pos), menu)
    }

    pub(in crate::game) fn lan_mod_craft_settle_at(
        &mut self,
        gi: usize,
        pos: Option<(i32, i32, i32)>,
        menu: &mut Menu,
    ) -> Option<usize> {
        let c = menu.mod_crafting()?;
        let guest = &self.server.lan.as_ref()?.hub.guests[gi];
        let open = guest.menu.as_ref()?;
        let grid = c.grid_slots().map(|k| menu.slot(k));
        let shown = menu.slot(c.result);
        let cached = open.craft.as_ref();
        let same_grid =
            cached.is_some_and(|cached| cached.grid.iter().map(Option::as_ref).eq(grid));
        if same_grid && cached.and_then(|cached| cached.result.as_ref()) == shown {
            return None;
        }
        let mut result = shown.cloned();
        let window = open.window;
        // Taking from the shown result is not a grid update. Do not build a new
        // preview over what the click left unless the inputs changed.
        if !same_grid {
            let (uuid, name) = (guest.uuid, guest.name.clone());
            self.native_prepare_item_craft(
                uuid,
                &name,
                pos.map(|(x, y, z)| [x, y, z]),
                &mut result,
            );
        }
        let changed = result.as_ref() != shown;
        if changed {
            menu.load_slot(c.result, result.clone());
        }
        let grid = c.grid_slots().map(|k| menu.slot(k).cloned()).collect();
        self.lan_mod_craft_store(gi, window, ModCraft { grid, result });
        changed.then_some(c.result)
    }

    /// Publish the result and the grid it belongs to as one value, before the hub syncs it.
    pub(in crate::game::server_side) fn lan_mod_craft_store(
        &mut self,
        gi: usize,
        window: i32,
        craft: ModCraft,
    ) {
        if let Some(host) = self.server.lan.as_ref() {
            self.journal_menu_click(host.hub.guests[gi].conn);
        }
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.guests.update(gi, |guest| {
                if let Some(open) = guest.menu.as_mut().filter(|open| open.window == window) {
                    open.craft = Some(craft);
                }
            });
        }
    }

    /// `CraftingMenu.slotsChanged` for one guest's mod crafting menu.
    pub(in crate::game) fn lan_mod_crafting_tick_one(&mut self, conn: u32) {
        let Some(gi) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.connection_index(conn))
        else {
            return;
        };
        self.lan_mod_crafting_tick_at(gi);
    }

    fn lan_mod_crafting_tick_at(&mut self, gi: usize) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let g = &host.hub.guests[gi];
        let Some((t, pos)) = g
            .menu
            .as_ref()
            .filter(|_| host.hub.here(g) && !self.player_tick_canceled(g.conn))
            .and_then(|m| match (m.mod_type, m.slots) {
                (Some(t), Slots::Block(pos)) if t.crafting.is_some() => Some((t, pos)),
                _ => None,
            })
        else {
            return;
        };
        let Some(BlockEntity::Mod(m)) = self.common.world.block_entities.get(&pos) else {
            return;
        };
        let mut items = m.items.clone();
        items.resize(t.slots, None);
        if Self::lan_mod_craft_shown(host, gi, &items).is_some() {
            return;
        }
        let mut menu = Menu::new(Kind::Mod(t));
        self.guest_menu_op(gi, &mut menu, |menu, env| {
            menu.load_container(env, &items, None);
        });
        self.lan_mod_craft_settle(gi, pos, &mut menu);
    }

    /// `CraftingMenu.slotsChanged` for the guests' mod crafting menus: the
    /// result each is shown follows its block entity's grid, which another
    /// player, a hopper or the mod may have changed, made again only when
    /// the grid changed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn lan_mod_crafting_tick(&mut self) {
        let Some(count) = self.server.lan.as_ref().map(|host| host.hub.guests.len()) else {
            return;
        };
        for gi in 0..count {
            self.lan_mod_crafting_tick_at(gi);
        }
    }

    /// `ServerPlayer.tick`'s `stillValid`: a guest's crafting table closes
    /// once it is gone or out of reach, giving its grid back.
    pub(in crate::game) fn lan_table_tick_one(&mut self, conn: u32) {
        let Some(window) = self.server.lan.as_ref().and_then(|host| {
            let (window, _, _, pos) = host.menus.get(&conn)?.crafting.table.as_ref()?;
            let g = host.hub.guest(conn).filter(|g| host.hub.here(g))?;
            (!self.lan_table_valid(g, *pos)).then_some(*window)
        }) else {
            return;
        };
        self.reply_menu_click(
            conn,
            sunrise_gameplay_common::game::lan_menu_data::ClickReply::Close(window),
        );
    }

    /// The whole crafting table menu for the client (`sendAllDataToRemote`).
    fn lan_send_table_content(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some((window, state, menu, _)) = host
            .menus
            .get_mut(&conn)
            .and_then(|p| p.crafting.table.as_mut())
        else {
            return;
        };
        *state = (*state + 1) & 0x7FFF;
        let (window, state) = (*window, *state);
        let mut items: Vec<Option<ItemStack>> = (0..10).map(|s| menu.slot(s).cloned()).collect();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        host.hub.send_menu_snapshot(gi, window, state, &items);
        host.hub.guests.update(gi, |g| g.inventory_told());
    }

    /// `AbstractContainerMenu.removed` of a crafting menu: the closed window
    /// (`window`; None closes the table whichever it is, and 0 the inventory
    /// menu) gives its grid back to the inventory, what doesn't fit is
    /// dropped. False when no such window is open.
    /// The open crafting table's own slots (result and grid) before the
    /// player's inventory, if the guest has one open.
    pub(in crate::game::server_side) fn lan_crafting_table_own_slots(
        &self,
        conn: u32,
    ) -> Option<usize> {
        let (_, _, menu, _) = self
            .server
            .lan
            .as_ref()?
            .menus
            .get(&conn)?
            .crafting
            .table
            .as_ref()?;
        Some(menu.slot_count() - 36)
    }

    /// Retire a game-owned menu on the guest record, even when their active
    /// cursor is temporarily in `Game` during a guest turn.
    pub(in crate::game) fn close_guest_menu_cursor(
        &mut self,
        gi: usize,
        window: i32,
    ) -> Option<ItemStack> {
        self.with_own_record(gi, |host, _, _| {
            host.hub.guests.update(gi, |g| g.close_menu_cursor(window))
        })
    }

    /// Return the cursor and inputs, or drop them if the player died or disconnected.
    pub(in crate::game) fn lan_crafting_removed(
        &mut self,
        conn: u32,
        window: Option<i32>,
        removal: MenuRemoval,
    ) -> bool {
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return false;
        };
        let crafting = &mut host.menus.entry(conn).or_default().crafting;
        let grid: Vec<ItemStack> = if window == Some(0) {
            *crafting.inventory = None;
            self.with_own_record(gi, |host, _, _| {
                Some(host.hub.guests.update(gi, |g| {
                    g.craft_result = None;
                    g.take_inventory_carried()
                        .into_iter()
                        .chain(g.craft_grid.iter_mut().filter_map(Option::take))
                        .collect()
                }))
            })
            .unwrap_or_default()
        } else {
            match crafting
                .table
                .take_if(|(w, ..)| window.is_none_or(|window| *w == window))
            {
                Some((window, _, menu, _)) => {
                    let carried = self.close_guest_menu_cursor(gi, window);
                    carried
                        .into_iter()
                        .chain((1..10).filter_map(|s| menu.slot(s).cloned()))
                        .collect()
                }
                None => return false,
            }
        };
        if !grid.is_empty() {
            if removal.drops_items() {
                self.drop_guest_items(gi, grid);
            } else {
                self.lan_place_back(gi, grid);
            }
        }
        true
    }

    /// `Inventory.placeItemBackInInventory` for the stacks a closing menu returned.
    /// Drop overflow after restoring the active turn, since spawning an item can call mods.
    pub(in crate::game) fn lan_place_back(&mut self, gi: usize, inputs: Vec<ItemStack>) {
        if inputs.is_empty() {
            return;
        }
        let overflow = self.with_own_record(gi, |lan, items, _| {
            Some(lan.hub.guests.update(gi, |guest| {
                let mut menu = Menu::new(Kind::Inventory);
                load_player(&mut menu, guest);
                place_back(&mut menu, items, inputs);
                store_player(&mut menu, guest);
                std::mem::take(&mut menu.dropped)
            }))
        });
        if let Some(overflow) = overflow.filter(|stacks| !stacks.is_empty()) {
            self.drop_guest_items(gi, overflow);
        }
    }

    /// A guest left: what their crafting grids and workstation held goes back into their
    /// inventory before it is saved (dropped where they stood if full).
    pub(in crate::game::server_side) fn lan_crafting_left(&mut self, g: &mut Guest) {
        // The guest's play state stays: their statistics are awarded and saved
        // after this (`HostEvent::Left`), which removes it.
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        // If the leaving level could not be opened for normal removal, put
        // its remaining cursors in the same fallback as the remaining inputs.
        let mut grid: Vec<_> = host.hub.take_closed_menu_cursors(g.conn).collect();
        let (proc_ret, proc_drop) = host.hub.take_closed_processing_inputs(g.conn);
        let mut dropped_processing = proc_ret;
        dropped_processing.extend(proc_drop);
        if let Some(menu) = g.menu.as_ref() {
            dropped_processing.extend(menu.take_processing_inputs(&mut g.native_menu));
        }
        grid.extend(g.take_menu_cursors());
        grid.extend(g.craft_grid.iter_mut().filter_map(Option::take));
        let play = host.menus.get_mut(&g.conn);
        g.craft_result = None;
        if let Some(play) = play {
            if let Some((_, _, menu, _)) = play.crafting.table.take() {
                grid.extend((1..10).filter_map(|s| menu.slot(s).cloned()));
            }
            // A workstation's inputs too.
            if let Some(station) = play.station.take() {
                grid.extend(super::lan_stations::station_inputs(&station));
            }
        }
        for stack in dropped_processing {
            self.spawn_item(stack, g.position, [0.0, 0.1, 0.0], 40);
        }
        if grid.is_empty() {
            return;
        }
        let mut menu = Menu::new(Kind::Inventory);
        load_player(&mut menu, g);
        place_back(&mut menu, self.items(), grid);
        let dropped = std::mem::take(&mut menu.dropped);
        store_player(&mut menu, g);
        for stack in dropped {
            self.spawn_item(stack, g.position, [0.0, 0.1, 0.0], 40);
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::equip_answers::EquipAnswersGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::item_smelted::GameHooks as _;

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt2 as _;

#[cfg(test)]
#[path = "lan_crafting/furnace_take_tests_server.rs"]
mod furnace_take_tests_server;
