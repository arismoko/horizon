//! Authoritative mod recipe-book placement on the active `Menu` and live storage.
use crate::game::ServerGame;
use mod_api::__private::game::Fields;
use mod_api::{
    ItemPlace, ItemResource, MenuSlotBacking, MenuSlotPlace, RecipeBookPlacement, ServerHost,
};
use std::collections::{BTreeMap, BTreeSet};
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
use sunrise_gameplay_common::game::lan_menu_data::{ClickReply, GuestMenuEffects};
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::{BlockEntity, ModMenuType};
use sunrise_sim::{
    lan::menu::Slots,
    menu::{
        Kind, Menu,
        place::{
            PostPlace, SlotCap,
            custom_plan::{MenuPlacement, PlacementRefusal},
        },
    },
    recipe_book::placement_sync as sync,
};

fn refusal(reason: PlacementRefusal) -> sync::Refusal {
    use sync::Refusal as R;
    match reason {
        PlacementRefusal::NotPlaceable => R::NotPlaceable,
        PlacementRefusal::InvalidLayout => R::Invalid,
        PlacementRefusal::Unavailable => R::Unavailable,
        PlacementRefusal::Denied => R::Denied,
    }
}

struct LiveOpening {
    gi: usize,
    uuid: u128,
    window: i32,
    instance: u64,
    kind: &'static ModMenuType,
    slots: Slots,
    creative: bool,
    hidden: Vec<usize>,
    owned_slots: bool,
}

impl ServerGame<'_> {
    pub(in crate::game::server_side) fn server_mod_book_place(
        &mut self,
        conn: u32,
        catalog: &sync::CatalogIdentity,
        display: &sync::DisplayId,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        let settings = self
            .common
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .ok_or(R::Unsupported)?;
        if settings.epoch != catalog.epoch || !settings.contains(catalog.book.as_str()) {
            return Err(R::Stale);
        }
        let authority = self
            .common
            .native_net
            .placement
            .servers
            .get(&conn)
            .ok_or(R::Unsupported)?;
        authority.peer.place(catalog)?;
        if !authority
            .displays
            .get(display.recipe.as_str())
            .is_some_and(|count| display.source < *count)
        {
            return Err(R::UnknownDisplay);
        }
        self.ensure_mod_book_displays();
        let index = self
            .common
            .recipe_tables
            .books
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .ok_or(R::Invalid)?;
        if index.generation() != catalog.generation {
            return Err(R::Stale);
        }
        let plan = index
            .placement(display.recipe.as_str())
            .map_err(refusal)?
            .clone();
        let opening = self.mod_book_live_opening(conn, catalog)?;
        let guest = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.get(opening.gi))
            .ok_or(R::Unavailable)?;
        let recipe = display.recipe.as_str();
        if !guest.recipe_book.knows(recipe) {
            return Err(R::Denied);
        }
        let name = guest.name.clone();
        self.native_player_recipe_book_click(opening.uuid, &name, recipe, use_max);
        self.apply_mod_book_plan(conn, catalog, display, &plan, use_max)
    }

    fn mod_book_live_opening(
        &self,
        conn: u32,
        catalog: &sync::CatalogIdentity,
    ) -> Result<LiveOpening, sync::Refusal> {
        use sync::Refusal as R;
        let settings = self
            .common
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .ok_or(R::Unsupported)?;
        if settings.epoch != catalog.epoch || !settings.contains(catalog.book.as_str()) {
            return Err(R::Stale);
        }
        if !self
            .common
            .native_net
            .placement
            .servers
            .get(&conn)
            .is_some_and(|server| server.peer.owns(catalog))
        {
            return Err(R::Stale);
        }
        let host = self.server.lan.as_ref().ok_or(R::Unavailable)?;
        let gi = host
            .hub
            .guests
            .connection_index(conn)
            .ok_or(R::Unavailable)?;
        let guest = &host.hub.guests[gi];
        if !host.hub.here(guest)
            || guest.spectator()
            || guest.dead
            || !self.guest_menu_still_valid(conn)
        {
            return Err(R::Unavailable);
        }
        let open = guest.menu.as_ref().ok_or(R::Unavailable)?;
        if guest.menu_generation != catalog.instance
            || open.window != catalog.window
            || open.registry_id.as_ref() != catalog.menu.as_str()
        {
            return Err(R::Stale);
        }
        let kind = open
            .mod_type
            .filter(|kind| kind.id == catalog.menu.as_str())
            .ok_or(R::Unavailable)?;
        if open.size != kind.slots {
            return Err(R::Unavailable);
        }
        let hidden = open.unlinked_slots(&self.common.world);
        Ok(LiveOpening {
            gi,
            uuid: guest.uuid,
            window: open.window,
            instance: guest.menu_generation,
            kind,
            slots: open.slots,
            creative: guest.creative(),
            hidden,
            owned_slots: kind.owned_slots.is_some(),
        })
    }

    pub fn apply_mod_book_plan(
        &mut self,
        conn: u32,
        catalog: &sync::CatalogIdentity,
        display: &sync::DisplayId,
        plan: &MenuPlacement,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        let opening = self.mod_book_live_opening(conn, catalog)?;
        if plan.awaits_grid() {
            let answer = self.ask_mod_book_place(
                opening.uuid,
                opening.kind.id,
                display.recipe.as_str(),
                use_max,
                opening.creative,
            )?;
            let current = self
                .mod_book_live_opening(conn, catalog)
                .map_err(|_| R::Changed)?;
            if !same_opening(&current, &opening) {
                return Err(R::Changed);
            }
            return match answer {
                RecipeBookPlacement::Nothing => Ok(PostPlace::Nothing),
                RecipeBookPlacement::Ghost => Ok(PostPlace::Ghost),
                RecipeBookPlacement::Grid {
                    width,
                    height,
                    slots,
                    clear,
                } => {
                    let grid_plan = self.resolve_mod_book_grid(
                        current.kind.id,
                        current.kind.slots,
                        &current.hidden,
                        display.recipe.as_str(),
                        plan,
                        width,
                        height,
                        &slots,
                        &clear,
                    )?;
                    self.apply_resolved_mod_book_place(conn, catalog, &current, &grid_plan, use_max)
                        .inspect_err(|reason| {
                            if matches!(reason, R::Invalid | R::Unavailable) {
                                log::warn!(
                                    "[recipe book] refused Grid placement for menu {} recipe {}: {reason:?}",
                                    current.kind.id,
                                    display.recipe.as_str()
                                );
                            }
                        })
                }
            };
        }
        self.apply_resolved_mod_book_place(conn, catalog, &opening, plan, use_max)
    }

    fn apply_resolved_mod_book_place(
        &mut self,
        conn: u32,
        catalog: &sync::CatalogIdentity,
        opening: &LiveOpening,
        plan: &MenuPlacement,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        match opening.slots {
            Slots::Block(pos) => {
                self.apply_block_mod_book_place(conn, catalog, opening, pos, plan, use_max)
            }
            Slots::Native if opening.owned_slots => {
                self.apply_owned_mod_book_place(conn, opening, plan, use_max)
            }
            Slots::Native => self.apply_backed_mod_book_place(conn, opening, plan, use_max),
            _ => Err(sync::Refusal::Unavailable),
        }
    }

    fn apply_block_mod_book_place(
        &mut self,
        conn: u32,
        catalog: &sync::CatalogIdentity,
        opening: &LiveOpening,
        pos: (i32, i32, i32),
        plan: &MenuPlacement,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        if !matches!(self.common.world.block_entities.get(&pos), Some(BlockEntity::Mod(m)) if !m.id().is_empty())
        {
            return Err(R::Unavailable);
        }
        // The state the placement is calculated against is read before the listeners that may
        // write (`recipe_sources`), and must be the same after them: an opening or a backing slot
        // they changed without closing the window is not this placement's any more.
        let state = self.block_book_state(opening)?;
        let raw_sources = self.lan_recipe_sources(opening.gi, opening.kind, pos);
        let current = self
            .mod_book_live_opening(conn, catalog)
            .map_err(|_| R::Changed)?;
        if !same_opening(&current, opening)
            || !matches!(self.common.world.block_entities.get(&pos), Some(BlockEntity::Mod(m)) if !m.id().is_empty())
        {
            return Err(R::Changed);
        }
        if self.block_book_state(&current)? != state {
            return Err(R::Changed);
        }
        let opening = current;
        let (before_main, main_backings, limits) = state;
        let mut cleared_blocks = BTreeSet::new();
        for &slot in plan.cleared_slots() {
            if opening.hidden.contains(&slot) {
                return Err(R::Unavailable);
            }
            let backing = main_backings
                .get(slot)
                .copied()
                .flatten()
                .ok_or(R::Unavailable)?;
            cleared_blocks.insert(backing);
        }
        let mut seen_sources = cleared_blocks;
        let mut source_keys = Vec::with_capacity(raw_sources.len());
        let mut stocks = Vec::with_capacity(raw_sources.len());
        for (spos, items) in &raw_sources {
            if self.common.world.block_entities.get(spos).is_none() {
                return Err(R::Changed);
            }
            let mut keys = Vec::with_capacity(items.len());
            let mut values = Vec::with_capacity(items.len());
            for (sslot, stack) in items.iter().enumerate() {
                let key = (*spos, sslot);
                let owned = seen_sources.insert(key);
                keys.push(owned.then_some(key));
                values.push(if owned { stack.clone() } else { None });
            }
            source_keys.push(keys);
            stocks.push(values);
        }
        let kind = Kind::Mod(opening.kind);
        let mut menu = self
            .server
            .lan
            .as_mut()
            .ok_or(R::Changed)?
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine
            .take()
            .filter(|(window, m)| *window == opening.window && m.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
        menu.reset_mod_visibility(opening.hidden.iter().copied());
        menu.set_registry_id(Some(opening.kind.id));
        menu.sources = stocks;
        let placed = self
            .guest_menu_edit(opening.gi, &mut menu, None, |menu, env| {
                menu.set_container_limits(limits);
                for (slot, stack) in before_main.iter().enumerate() {
                    menu.load_slot(slot, stack.clone());
                }
                menu.place_custom_recipe(env, plan, use_max, opening.creative)
            })
            .ok_or(R::Changed)?;
        let after_sources = std::mem::take(&mut menu.sources);
        let post = match placed {
            Ok(post) => post,
            Err(reason) => {
                let _ = GuestMenuEffects::take(&mut menu);
                if let Some(host) = self.server.lan.as_mut() {
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((opening.window, menu));
                }
                return Err(refusal(reason));
            }
        };
        let mut updated_blocks = BTreeMap::new();
        for &slot in plan.cleared_slots() {
            let after = menu.slot(slot).cloned();
            if before_main[slot] != after
                && let Some(key) = main_backings[slot]
            {
                updated_blocks.insert(key, after);
            }
        }
        for ((keys, (_, before_items)), after_items) in
            source_keys.iter().zip(&raw_sources).zip(after_sources)
        {
            for ((key, before_stack), after_stack) in keys.iter().zip(before_items).zip(after_items)
            {
                if let Some(key) = key
                    && *before_stack != after_stack
                {
                    updated_blocks.insert(*key, after_stack);
                }
            }
        }
        for (slot, backing) in main_backings.iter().enumerate() {
            if let Some(key) = backing
                && let Some(stack) = updated_blocks.get(key)
            {
                menu.load_slot(slot, stack.clone());
            }
        }
        let mut changed_blocks = BTreeSet::new();
        for ((bpos, bslot), stack) in updated_blocks {
            if let Some(entity) = self.common.world.block_entities.get_mut(&bpos) {
                if let BlockEntity::Mod(m) = entity
                    && bpos == pos
                    && m.items.len() <= bslot
                    && bslot < opening.kind.slots
                {
                    m.items.resize(opening.kind.slots, None);
                }
                if let Some(target) = entity.slot_mut(bslot) {
                    *target = stack;
                    changed_blocks.insert(bpos);
                }
            }
        }
        for bpos in changed_blocks {
            self.container_changed(bpos);
        }
        let effects = GuestMenuEffects::take(&mut menu);
        self.lan_mod_craft_settle_at(opening.gi, Some(pos), &mut menu);
        if let Some(host) = self.server.lan.as_mut() {
            *host.menus.entry(conn).or_default().crafting.machine = Some((opening.window, menu));
        }
        self.publish_guest_menu_effects(conn, effects);
        Ok(post)
    }

    fn apply_owned_mod_book_place(
        &mut self,
        conn: u32,
        opening: &LiveOpening,
        plan: &MenuPlacement,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        let result_slot = opening.kind.crafting.map(|c| c.result);
        for &slot in plan.cleared_slots() {
            if slot >= opening.kind.slots
                || opening.hidden.contains(&slot)
                || result_slot == Some(slot)
            {
                return Err(R::Unavailable);
            }
        }
        let before_main = {
            let host = self.server.lan.as_ref().ok_or(R::Changed)?;
            let guest = host.hub.guests.get(opening.gi).ok_or(R::Changed)?;
            opening
                .slots
                .read(&self.common.world, guest)
                .filter(|items| items.len() == opening.kind.slots)
                .ok_or(R::Unavailable)?
        };
        let kind = Kind::Mod(opening.kind);
        let mut menu = self
            .server
            .lan
            .as_mut()
            .ok_or(R::Changed)?
            .menus
            .entry(conn)
            .or_default()
            .owned_menu
            .take()
            .filter(|(generation, m)| *generation == opening.instance && m.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
        menu.reset_mod_visibility(opening.hidden.iter().copied());
        menu.set_registry_id(Some(opening.kind.id));
        let placed = self
            .guest_menu_edit(opening.gi, &mut menu, None, |menu, env| {
                for (slot, stack) in before_main.iter().enumerate() {
                    menu.load_slot(slot, stack.clone());
                }
                menu.place_custom_recipe(env, plan, use_max, opening.creative)
            })
            .ok_or(R::Changed)?;
        let post = match placed {
            Ok(post) => post,
            Err(reason) => {
                let _ = GuestMenuEffects::take(&mut menu);
                if let Some(host) = self.server.lan.as_mut() {
                    host.menus.entry(conn).or_default().owned_menu = Some((opening.instance, menu));
                }
                return Err(refusal(reason));
            }
        };
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.guests.update(opening.gi, |guest| {
                for &slot in plan.cleared_slots() {
                    if let Some(target) = guest.native_menu.get_mut(slot) {
                        *target = menu.slot(slot).cloned();
                    }
                }
            });
        }
        let effects = GuestMenuEffects::take(&mut menu);
        self.lan_mod_craft_settle_at(opening.gi, None, &mut menu);
        if let Some(host) = self.server.lan.as_mut() {
            host.menus.entry(conn).or_default().owned_menu = Some((opening.instance, menu));
        }
        self.publish_guest_menu_effects(conn, effects);
        Ok(post)
    }

    fn apply_backed_mod_book_place(
        &mut self,
        conn: u32,
        opening: &LiveOpening,
        plan: &MenuPlacement,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        let menu_ref = self
            .native_menu_ref(opening.uuid, opening.window)
            .ok_or(R::Unavailable)?;
        let player = menu_ref.fields().player;
        if !self.held_places_bound(conn, player) {
            self.reply_menu_click(conn, ClickReply::Close(opening.window));
            return Err(R::Stale);
        }
        let places: Vec<Option<MenuSlotPlace>> = (0..opening.kind.slots)
            .map(|slot| self.native_menu_place(conn, opening.window, slot))
            .collect();
        let mut seen_places = Vec::<MenuSlotPlace>::new();
        for place in places.iter().flatten().copied() {
            if seen_places.contains(&place) {
                return Err(R::Unavailable);
            }
            seen_places.push(place);
        }
        for &slot in plan.cleared_slots() {
            if opening.hidden.contains(&slot) || places.get(slot).copied().flatten().is_none() {
                return Err(R::Unavailable);
            }
        }
        // Read live handler stacks directly rather than the guest's cached native_menu mirror.
        let mut before_api = Vec::with_capacity(opening.kind.slots);
        {
            let host: &mut dyn ServerHost = self;
            for (slot, place) in places.iter().copied().enumerate() {
                if opening.hidden.contains(&slot) {
                    before_api.push(None);
                    continue;
                }
                match place {
                    Some(place) => {
                        if !place.resolves(host, player)
                            || menu_ref.slot_backing(host, slot)
                                != Some(MenuSlotBacking::Place(place))
                        {
                            return Err(R::Unavailable);
                        }
                        let stack = ItemPlace::MenuSlot {
                            menu: menu_ref,
                            slot,
                        }
                        .with_stack(host, |stack| stack.project())
                        .flatten();
                        before_api.push(stack);
                    }
                    None => {
                        if !matches!(
                            menu_ref.slot_backing(host, slot),
                            Some(MenuSlotBacking::Native(_))
                        ) {
                            return Err(R::Unavailable);
                        }
                        before_api.push(None);
                    }
                }
            }
        }
        let before_main: Vec<Option<ItemStack>> = before_api
            .iter()
            .map(|stack| {
                stack
                    .as_ref()
                    .map(|stack| {
                        stack
                            .try_into_engine(self.items())
                            .map_err(|_| R::Unavailable)
                    })
                    .transpose()
            })
            .collect::<Result<_, _>>()?;
        let kind = Kind::Mod(opening.kind);
        let mut menu = self
            .server
            .lan
            .as_mut()
            .ok_or(R::Changed)?
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine
            .take()
            .filter(|(window, m)| *window == opening.window && m.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
        menu.reset_mod_visibility(opening.hidden.iter().copied());
        menu.set_registry_id(Some(opening.kind.id));
        let mut synced_places = false;
        let placed = self.storage_transaction(|game, tx| {
            game.journal_parked_player(opening.gi);
            // Two phases. The planner first names the (slot, stack) pairs it will weigh, with no
            // host. The game then asks each backed slot's live handler what it takes of that stack
            // (NeoForge `ResourceHandlerSlot.mayPlace` and `getMaxStackSize`), and the placement
            // takes those answers, so a handler that fits one stack gets one craft, not a plan it
            // would roll back whole.
            let candidates = game
                .guest_menu_edit(opening.gi, &mut menu, None, |menu, env| {
                    for (slot, stack) in before_main.iter().enumerate() {
                        menu.load_slot(slot, stack.clone());
                    }
                    menu.custom_recipe_candidates(env, plan)
                })
                .ok_or(R::Changed)?
                .map_err(refusal)?;
            let mut caps = Vec::new();
            for (slot, prototype) in candidates {
                if places.get(slot).copied().flatten().is_none() {
                    continue;
                }
                let api = prototype.try_into_api().map_err(|_| R::Unavailable)?;
                let max = match game.menu_probe_slot_holds(menu_ref, slot, &api) {
                    Ok(Some(max)) => max,
                    Ok(None) | Err(_) => return Err(R::Unavailable),
                };
                caps.push(SlotCap {
                    slot,
                    prototype,
                    max,
                });
            }
            let post = game
                .guest_menu_edit(opening.gi, &mut menu, None, |menu, env| {
                    for (slot, stack) in before_main.iter().enumerate() {
                        menu.load_slot(slot, stack.clone());
                    }
                    menu.place_custom_recipe_capped(env, plan, use_max, opening.creative, &caps)
                })
                .ok_or(R::Changed)?
                .map_err(refusal)?;
            let mut writes = Vec::new();
            for &slot in plan.cleared_slots() {
                let after = menu.slot(slot).cloned();
                if before_main[slot] == after {
                    continue;
                }
                let backing = places[slot].ok_or(R::Changed)?;
                let after_api = after
                    .as_ref()
                    .map(TryIntoApi::try_into_api)
                    .transpose()
                    .map_err(|_| R::Changed)?;
                writes.push((slot, backing, before_api[slot].clone(), after_api));
            }
            synced_places = !writes.is_empty();
            let host: &mut dyn ServerHost = game;
            for (slot, backing, before_stack, after_stack) in &writes {
                if menu_ref.slot_backing(host, *slot) != Some(MenuSlotBacking::Place(*backing)) {
                    return Err(R::Changed);
                }
                let place = ItemPlace::MenuSlot {
                    menu: menu_ref,
                    slot: *slot,
                };
                if let Some(stack) = before_stack {
                    let resource = ItemResource::of(stack);
                    if place.extract(host, tx, &resource, stack.count()) != Ok(stack.count()) {
                        return Err(R::Changed);
                    }
                }
                if let Some(stack) = after_stack {
                    let resource = ItemResource::of(stack);
                    if place.insert(host, tx, &resource, stack.count()) != Ok(stack.count()) {
                        return Err(R::Changed);
                    }
                }
            }
            Ok(post)
        });
        let post = match placed {
            Ok(post) => post,
            Err(reason) => {
                let _ = GuestMenuEffects::take(&mut menu);
                if let Some(host) = self.server.lan.as_mut() {
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((opening.window, menu));
                }
                return Err(reason);
            }
        };
        let effects = GuestMenuEffects::take(&mut menu);
        self.lan_mod_craft_settle_at(opening.gi, None, &mut menu);
        if let Some(host) = self.server.lan.as_mut() {
            *host.menus.entry(conn).or_default().crafting.machine = Some((opening.window, menu));
        }
        self.publish_guest_menu_effects(conn, effects);
        if synced_places {
            self.publish_backed_menu_click(conn, opening.uuid, menu_ref, opening.window);
        }
        Ok(post)
    }

    fn block_book_state(&self, opening: &LiveOpening) -> Result<BlockBookState, sync::Refusal> {
        use sync::Refusal as R;
        let host = self.server.lan.as_ref().ok_or(R::Changed)?;
        let guest = host.hub.guests.get(opening.gi).ok_or(R::Changed)?;
        let before_main = opening
            .slots
            .read(&self.common.world, guest)
            .filter(|items| items.len() == opening.kind.slots)
            .ok_or(R::Unavailable)?;
        let mut seen_main = BTreeSet::new();
        let mut main_backings = Vec::with_capacity(opening.kind.slots);
        let mut limits = Vec::with_capacity(opening.kind.slots);
        for slot in 0..opening.kind.slots {
            match opening
                .slots
                .held_block_slot(&self.common.world, guest, slot)
            {
                Some((bpos, bslot)) => {
                    if !seen_main.insert((bpos, bslot))
                        || self.common.world.block_entities.get(&bpos).is_none()
                    {
                        return Err(R::Unavailable);
                    }
                    main_backings.push(Some((bpos, bslot)));
                    limits.push(self.common.world.container_stack_limit(bpos));
                }
                None => {
                    main_backings.push(None);
                    limits.push(None);
                }
            }
        }
        Ok((before_main, main_backings, limits))
    }
}

/// A live opening's identity: the guest and window it is for, the menu and where its slots are.
fn same_opening(a: &LiveOpening, b: &LiveOpening) -> bool {
    a.gi == b.gi
        && a.uuid == b.uuid
        && a.window == b.window
        && a.instance == b.instance
        && std::ptr::eq(a.kind, b.kind)
        && a.slots == b.slots
        && a.creative == b.creative
        && a.hidden == b.hidden
        && a.owned_slots == b.owned_slots
}

/// A block menu's main slots as a placement reads them: their stacks, the block slot backing
/// each and its stack limit.
type BlockBookState = (
    Vec<Option<ItemStack>>,
    Vec<Option<((i32, i32, i32), usize)>>,
    Vec<Option<u32>>,
);

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_book_displays::NativeBookDisplaysGameExt1 as _;
