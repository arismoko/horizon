//! Server-side `menu_slot_changed`: `AbstractContainerMenu.slotsChanged` for mod menu types.
//!
//! The contract (a Sunrise design, stated in the hook's docs): one call per batch of changes to
//! the non-result slots of an open mod menu's own container, never per slot.
//! - A click reports each click step's changes (a multi-slot drag, a swap, a pickup-all: one
//!   step, one batch; a take that refills inputs: one more batch) inside the click's transaction,
//!   so a failed listener or an invalid `MenuSlotWrite` undoes the whole click.
//! - A committed host write (`ItemPlace::MenuSlot`, `ItemPlace::BlockEntity`, `BlockEntityRef`
//!   writes) is reported once per outermost committed transaction, never on abort, in ascending
//!   menu generation and then block position order.
//! - A write outside any transaction to the block entity behind an open block-backed mod menu (a
//!   hopper, `/item replace block`) is reported at the write. Vanilla's block containers never
//!   notify an open menu; Sunrise does so the menu's outputs can follow its inputs.
//!
//! Within a batch the changes are in ascending menu slot order. Viewers see listener outputs from a
//! click in the click's reply, and from any other write at the next `broadcastChanges`, as for any
//! container write. Outputs never report again.
use mod_api::{MenuContainer, MenuSlotChange, MenuSlotContext, MenuSlotWrite};
use sunrise_gameplay_common::game::api_values::{StackReadError, TryIntoApi, TryIntoEngine};
use sunrise_gameplay_common::game::transactions::After;
use sunrise_items::items::{ItemStack, Items};
use sunrise_level::{
    block_entities::Pos,
    containers::{BlockEntity, ModMenuType},
};
use sunrise_sim::{
    lan::{guest::Guest, guest_list::GuestList, menu::Slots},
    menu::Menu,
};

/// Maximum stack size for container slots without an explicit per-slot owned limit
/// (`Container.DEFAULT_MAX_STACK_SIZE`).
const CONTAINER_MAX: u32 = 99;

/// Open mod menu whose `menu_slot_changed` dispatch is currently on the call stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum SlotChangedTarget {
    Owned(u64),
    Block(Pos),
}

/// Where an open mod menu's own slots live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum SlotChangedBacking {
    /// A blockless owned menu's transient slots, for the opening with this menu generation.
    Owned { generation: u64 },
    /// The block entity at `pos`, of the generation the menu was admitted on.
    Block { pos: Pos, generation: Option<u64> },
}

/// The open mod menu a `menu_slot_changed` call is about, and the guest whose turn it runs in.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct SlotChangedMenu {
    pub(in crate::game) conn: u32,
    pub(in crate::game) uuid: u128,
    pub(in crate::game) window: i32,
    pub(in crate::game) layout: &'static ModMenuType,
    pub(in crate::game) backing: SlotChangedBacking,
}

impl SlotChangedMenu {
    fn target(&self) -> SlotChangedTarget {
        match self.backing {
            SlotChangedBacking::Owned { generation } => SlotChangedTarget::Owned(generation),
            SlotChangedBacking::Block { pos, .. } => SlotChangedTarget::Block(pos),
        }
    }

    fn pos(&self) -> Option<[i32; 3]> {
        match self.backing {
            SlotChangedBacking::Owned { .. } => None,
            SlotChangedBacking::Block { pos, .. } => Some([pos.0, pos.1, pos.2]),
        }
    }
}

/// The menu's own container before and after one click step or one write.
#[derive(Clone, Copy)]
pub(in crate::game) struct SlotSnapshots<'a> {
    pub(in crate::game) before: &'a [Option<ItemStack>],
    pub(in crate::game) after: &'a [Option<ItemStack>],
}

/// A slot whose stack the mod API can't carry (an item with no API key, or a component patch the
/// API rejects), so the batch it belongs to can't be reported. A click with one is refused; any
/// other write has already happened and goes unreported, with this reason logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::game) struct UnreportableSlot {
    pub(in crate::game) container: MenuContainer,
    pub(in crate::game) slot: usize,
    pub(in crate::game) reason: StackReadError,
}

struct SlotChangedGuard<'a, 'g> {
    game: &'a mut crate::game::ServerGame<'g>,
}

impl Drop for SlotChangedGuard<'_, '_> {
    fn drop(&mut self) {
        self.game.server.slot_changed_active.pop();
    }
}

/// What a slot holds for reporting: a mod may leave a count-0 stack in its own slots, which is
/// empty, as `machines::get` reads it.
fn held(items: &[Option<ItemStack>], slot: usize) -> Option<&ItemStack> {
    items
        .get(slot)
        .and_then(Option::as_ref)
        .filter(|stack| stack.count > 0)
}

/// `stack` as the API carries it, or why it can't be reported.
fn reportable(
    stack: Option<&ItemStack>,
    container: MenuContainer,
    slot: usize,
) -> Result<Option<mod_api::ItemStack>, UnreportableSlot> {
    stack
        .map(TryIntoApi::try_into_api)
        .transpose()
        .map_err(|reason| UnreportableSlot {
            container,
            slot,
            reason,
        })
}

/// The non-result slots of `layout`'s own container that differ between the snapshots, as one
/// batch in ascending menu slot order. Result, crafting-result and linked slots never count.
pub(in crate::game::server_side) fn collect_slot_changes(
    layout: &ModMenuType,
    snapshots: SlotSnapshots<'_>,
) -> Result<Vec<MenuSlotChange>, UnreportableSlot> {
    let mut changes = Vec::new();
    for slot in 0..layout.slots {
        if !layout.owns_slot(slot)
            || layout.results.contains(&slot)
            || layout.crafting.is_some_and(|c| c.result == slot)
        {
            continue;
        }
        let (old, new) = (held(snapshots.before, slot), held(snapshots.after, slot));
        if old == new {
            continue;
        }
        let (container, index) = match layout.crafting {
            Some(c) if c.grid_slots().contains(&slot) => (MenuContainer::Grid, slot - c.grid),
            _ => (MenuContainer::Main, slot),
        };
        changes.push(MenuSlotChange::new(
            container,
            index,
            reportable(old, container, index)?,
            reportable(new, container, index)?,
        ));
    }
    Ok(changes)
}

/// Owned snapshot of a menu's own containers used to lend a read-only [`MenuSlotContext`] across
/// a `menu_slot_changed` callback.
pub(in crate::game) struct SlotChangedSnapshot {
    pub(in crate::game) player: u128,
    pub(in crate::game) pos: Option<[i32; 3]>,
    main: Vec<Option<mod_api::ItemStack>>,
    grid: Vec<Option<mod_api::ItemStack>>,
    result: Vec<Option<mod_api::ItemStack>>,
}

impl SlotChangedSnapshot {
    pub(in crate::game::server_side) fn with_context<R>(
        &self,
        f: impl FnOnce(&MenuSlotContext<'_>) -> R,
    ) -> R {
        let read = |container: MenuContainer, index: usize| match container {
            MenuContainer::Main => self.main.get(index).cloned().flatten(),
            MenuContainer::Grid => self.grid.get(index).cloned().flatten(),
            MenuContainer::Result => self.result.get(index).cloned().flatten(),
            MenuContainer::Player => None,
            _ => None,
        };
        let context = MenuSlotContext::new(Some(self.player), self.pos, &read);
        f(&context)
    }
}

/// Build a read-only [`SlotChangedSnapshot`] from a `Menu`'s own containers. A slot the API can't
/// carry refuses the snapshot rather than reading as empty.
pub(in crate::game::server_side) fn context_from_menu(
    menu: &Menu,
    player: u128,
    pos: Option<[i32; 3]>,
) -> Result<SlotChangedSnapshot, UnreportableSlot> {
    let copy = |slots: &[Option<ItemStack>], container| {
        (0..slots.len())
            .map(|slot| reportable(held(slots, slot), container, slot))
            .collect::<Result<Vec<_>, _>>()
    };
    let (main, grid, result) = menu.slot_containers();
    Ok(SlotChangedSnapshot {
        player,
        pos,
        main: copy(main, MenuContainer::Main)?,
        grid: copy(grid, MenuContainer::Grid)?,
        result: copy(result, MenuContainer::Result)?,
    })
}

/// Why [`validate_slot_writes`] refused a requested [`MenuSlotWrite`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotWriteRefusal {
    NonMainContainer,
    UnavailableSlot,
    CraftingSlot,
    OwnedSlotBounds,
    NonResultBlockSlot,
    InvalidStack,
    StackLimitExceeded,
}

impl SlotWriteRefusal {
    const fn reason(self) -> &'static str {
        match self {
            Self::NonMainContainer => "output write must target MenuContainer::Main",
            Self::UnavailableSlot => "output write targets an undeclared or linked slot",
            Self::CraftingSlot => "output write cannot target a crafting slot",
            Self::OwnedSlotBounds => "output write is outside owned_slots",
            Self::NonResultBlockSlot => {
                "block-backed output write must target a declared result slot"
            }
            Self::InvalidStack => "output write carries an invalid stack",
            Self::StackLimitExceeded => "output write exceeds the slot or item stack limit",
        }
    }
}

/// Which of a menu's own slots a `menu_slot_changed` listener may write, and how much each holds.
#[derive(Clone, Copy, Debug)]
enum WriteScope<'a> {
    /// A blockless owned menu: any declared owned `Main` slot, up to its declared limit.
    Owned(&'a [u8]),
    /// A block-backed menu: only its declared `results` slots, up to the block's container limit.
    Block(Option<u32>),
}

/// Validate every requested [`MenuSlotWrite`] before applying any of them.
fn validate_slot_writes(
    items: &Items,
    max_stack: impl Fn(&ItemStack) -> u32,
    layout: &ModMenuType,
    scope: WriteScope<'_>,
    outputs: Vec<MenuSlotWrite>,
) -> Result<Vec<(usize, Option<ItemStack>)>, SlotWriteRefusal> {
    let mut validated = Vec::with_capacity(outputs.len());
    for write in outputs {
        if write.container != MenuContainer::Main {
            return Err(SlotWriteRefusal::NonMainContainer);
        }
        let slot = write.slot;
        if slot >= layout.slots || !layout.owns_slot(slot) {
            return Err(SlotWriteRefusal::UnavailableSlot);
        }
        if layout
            .crafting
            .is_some_and(|c| c.grid_slots().contains(&slot) || c.result == slot)
        {
            return Err(SlotWriteRefusal::CraftingSlot);
        }
        let slot_cap = match scope {
            WriteScope::Owned(limits) => {
                let Some(&limit) = limits.get(slot) else {
                    return Err(SlotWriteRefusal::OwnedSlotBounds);
                };
                u32::from(limit)
            }
            WriteScope::Block(block_limit) => {
                if !layout.results.contains(&slot) {
                    return Err(SlotWriteRefusal::NonResultBlockSlot);
                }
                block_limit.unwrap_or(CONTAINER_MAX)
            }
        };
        let stack = match write.stack {
            None => None,
            Some(api_stack) => {
                let engine_stack = api_stack
                    .try_into_engine(items)
                    .map_err(|_| SlotWriteRefusal::InvalidStack)?;
                let limit = slot_cap.min(max_stack(&engine_stack));
                if engine_stack.count == 0 || engine_stack.count > limit {
                    return Err(SlotWriteRefusal::StackLimitExceeded);
                }
                Some(engine_stack)
            }
        };
        validated.push((slot, stack));
    }
    Ok(validated)
}

fn resized_items(items: &[Option<ItemStack>], len: usize) -> Vec<Option<ItemStack>> {
    let mut out = items.to_vec();
    out.resize(len, None);
    out
}

fn log_unreportable(menu: &SlotChangedMenu, refusal: &UnreportableSlot) {
    log::warn!(
        "menu_slot_changed: {} {:?} slot {} holds a stack the mod API can't carry ({:?}); the change is not reported",
        menu.layout.id,
        refusal.container,
        refusal.slot,
        refusal.reason
    );
}

impl crate::game::ServerGame<'_> {
    /// `menu_slot_changed` for one batch of `menu`'s non-result slot changes: inside a click step
    /// (`live` is the click's lifted `Menu`), or for a committed or external write (`None`).
    /// Validated outputs go into the menu's own storage (and `live`) without reporting again, and
    /// the menu's last reported snapshot becomes the result. `false` means the caller undoes its
    /// action: a listener failed, an output was invalid, or a slot can't be reported.
    pub(in crate::game) fn mod_menu_slot_changed(
        &mut self,
        menu: &SlotChangedMenu,
        live: Option<&mut Menu>,
        snapshots: SlotSnapshots<'_>,
    ) -> bool {
        let changed = match collect_slot_changes(menu.layout, snapshots) {
            Ok(changed) => changed,
            Err(refusal) => {
                log_unreportable(menu, &refusal);
                return false;
            }
        };
        if changed.is_empty() {
            self.record_slot_changed_seen(menu);
            return true;
        }
        let context = match self.slot_changed_context(menu, snapshots.after, live.as_deref()) {
            Ok(context) => context,
            Err(refusal) => {
                log_unreportable(menu, &refusal);
                return false;
            }
        };
        // Outputs land inside the guard too, so writing them reports nothing again.
        let outcome = self.with_slot_changed_guard(menu.target(), |game| {
            let writes = game.fire_slot_changed_for_guest(menu, &changed, &context)?;
            game.apply_slot_writes(menu, &writes, live);
            Some(())
        });
        // A listener writing the menu it hears is reported once, by the outer call.
        outcome.is_none_or(|fired| fired.is_some())
    }

    /// Build a read-only [`MenuSlotContext`] through [`context_from_menu`] on every path: the
    /// live lifted `Menu` during a click, or a read-only `Menu::new(Kind::Mod(layout))` loaded
    /// with the live container items (including linked slots and the recomputed crafting result),
    /// so every path presents listeners the same snapshot.
    fn slot_changed_context(
        &self,
        menu: &SlotChangedMenu,
        items: &[Option<ItemStack>],
        live: Option<&Menu>,
    ) -> Result<SlotChangedSnapshot, UnreportableSlot> {
        #[cfg(any(test, feature = "test-support"))]
        probe::context();
        if let Some(live) = live {
            return context_from_menu(live, menu.uuid, menu.pos());
        }
        let layout = menu.layout;
        let guest = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(menu.conn));
        let mut full_items = resized_items(items, layout.slots);
        if let Some(l) = layout.links
            && let Some(links) = guest
                .and_then(|g| g.menu.as_ref())
                .map(|m| m.links.as_slice())
        {
            for (k, stack) in l.slots().zip(
                sunrise_sim::lan::menu::read_links(&self.common.world, links, l.count)
                    .into_iter()
                    .chain(std::iter::repeat(None)),
            ) {
                full_items[k] = stack;
            }
        }
        let mut lifted = Menu::new(sunrise_sim::menu::Kind::Mod(layout));
        let mut env = self.menu_env();
        env.loop_context = &self.common.loop_context;
        if let Some(host) = self.server.lan.as_ref()
            && let Some(guest) = guest
        {
            env.player = Some(guest.uuid);
            env.player_entity_id = Some(guest.entity_id);
            env.limited_crafting = host.hub.limited_crafting;
            env.known_recipes = Some(&guest.recipe_book);
        }
        lifted.load_container(&env, &full_items, None);
        context_from_menu(&lifted, menu.uuid, menu.pos())
    }

    fn with_slot_changed_guard<R>(
        &mut self,
        target: SlotChangedTarget,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if self.server.slot_changed_active.contains(&target) {
            return None;
        }
        self.server.slot_changed_active.push(target);
        let guard = SlotChangedGuard { game: self };
        Some(f(guard.game))
    }

    /// Invoke `menu_slot_changed` as `menu`'s guest and return the validated writes, or `None`
    /// on a listener panic or an invalid write.
    fn fire_slot_changed_for_guest(
        &mut self,
        menu: &SlotChangedMenu,
        changed: &[MenuSlotChange],
        context: &SlotChangedSnapshot,
    ) -> Option<Vec<(usize, Option<ItemStack>)>> {
        let conn = menu.conn;
        if self.server.lan_acting == Some(conn) || self.server.lan_guest_swaps.contains(&conn) {
            self.fire_validated_slot_changed(menu, changed, context)
        } else {
            self.as_guest(conn, |game| {
                game.fire_validated_slot_changed(menu, changed, context)
            })
            .flatten()
        }
    }

    fn fire_validated_slot_changed(
        &mut self,
        menu: &SlotChangedMenu,
        changed: &[MenuSlotChange],
        context: &SlotChangedSnapshot,
    ) -> Option<Vec<(usize, Option<ItemStack>)>> {
        let menu_id = menu.layout.id;
        let mut outputs = Vec::new();
        let failures = mod_api::runtime::isolated_failures();
        context.with_context(|context| {
            self.native_menu_slot_changed(mod_api::__private::MenuSlotChangedCall {
                menu: menu_id,
                changed,
                context,
                outputs: &mut outputs,
            });
        });
        if mod_api::runtime::isolated_failures() != failures {
            log::error!("menu_slot_changed: a listener failed for {menu_id}; the change is undone");
            return None;
        }
        let scope = match menu.backing {
            SlotChangedBacking::Owned { .. } => {
                WriteScope::Owned(menu.layout.owned_slots.as_deref().unwrap_or_default())
            }
            SlotChangedBacking::Block { pos, .. } => {
                WriteScope::Block(self.common.world.container_stack_limit(pos))
            }
        };
        match validate_slot_writes(
            self.items(),
            |stack| self.containers().max_stack(stack),
            menu.layout,
            scope,
            outputs,
        ) {
            Ok(writes) => Some(writes),
            Err(refusal) => {
                log::warn!(
                    "menu_slot_changed: refused output write for {menu_id}: {}",
                    refusal.reason()
                );
                None
            }
        }
    }

    /// Write validated outputs into `menu`'s own storage and `live`, then remember what the menu
    /// holds as its last reported snapshot. An owned menu's storage is the opening's, so a menu a
    /// listener closed or replaced keeps nothing; a block's is the block entity's, kept while it
    /// is the one the menu was admitted on.
    fn apply_slot_writes(
        &mut self,
        menu: &SlotChangedMenu,
        writes: &[(usize, Option<ItemStack>)],
        live: Option<&mut Menu>,
    ) {
        let slots = menu.layout.slots;
        let stored = match menu.backing {
            SlotChangedBacking::Owned { generation } => {
                if let Some(player) = self.player(menu.uuid) {
                    self.journal_menus(player);
                }
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                let Some(gi) = host.hub.guests.connection_index(menu.conn) else {
                    return;
                };
                let guest = &host.hub.guests[gi];
                if guest.menu_generation != generation
                    || guest.menu.as_ref().is_none_or(|m| m.window != menu.window)
                {
                    return;
                }
                // Host writes the listener made to the transient slots, then its outputs.
                let mut stored = resized_items(&guest.native_menu, slots);
                for (slot, stack) in writes {
                    stored[*slot] = stack.clone();
                }
                host.hub
                    .guests
                    .update(gi, |g| g.native_menu = stored.clone());
                stored
            }
            SlotChangedBacking::Block { pos, generation } => {
                if generation != self.common.world.block_entities.generation(&pos) {
                    return;
                }
                if !writes.is_empty() {
                    self.journal_block_entity(pos);
                }
                let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get_mut(&pos)
                else {
                    return;
                };
                if entity.items.len() < slots {
                    entity.items.resize(slots, None);
                }
                let mut mutated = false;
                for (slot, stack) in writes {
                    if entity.items[*slot] != *stack {
                        entity.items[*slot] = stack.clone();
                        mutated = true;
                    }
                }
                let stored = entity.items.clone();
                if mutated {
                    self.after_commit(After::Changed(pos));
                }
                stored
            }
        };
        if let Some(live) = live {
            for slot in 0..slots {
                if menu.layout.owns_slot(slot)
                    && menu.layout.crafting.is_none_or(|c| c.result != slot)
                {
                    live.load_slot(slot, stored.get(slot).cloned().flatten());
                }
            }
        }
        self.record_slot_changed_seen(menu);
    }

    /// Remember what `menu`'s own storage holds now as its last reported snapshot (every listening
    /// viewer's, for a block), journaled inside a transaction so a rollback restores the previous.
    fn record_slot_changed_seen(&mut self, menu: &SlotChangedMenu) {
        match menu.backing {
            SlotChangedBacking::Owned { generation } => {
                let Some(seen) = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(menu.conn))
                    .filter(|g| g.menu_generation == generation)
                    .map(|g| resized_items(&g.native_menu, menu.layout.slots))
                else {
                    return;
                };
                self.record_owned_slot_changed_seen(menu.conn, generation, seen);
            }
            SlotChangedBacking::Block { pos, .. } => {
                let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get(&pos)
                else {
                    return;
                };
                let items = entity.items.clone();
                self.record_block_slot_changed_seen(pos, &items);
            }
        }
    }

    fn record_owned_slot_changed_seen(
        &mut self,
        conn: u32,
        generation: u64,
        items: Vec<Option<ItemStack>>,
    ) {
        if !self.common.transactions.is_empty() {
            self.journal_menu_click(conn);
        }
        if let Some(host) = self.server.lan.as_mut()
            && let Some(gi) = host.hub.guests.connection_index(conn)
            && host.hub.guests[gi].menu_generation == generation
        {
            host.hub.guests.update(gi, |g| {
                if let Some(open) = g.menu.as_mut() {
                    open.slot_changed_seen = Some(items);
                }
            });
        }
    }

    fn record_block_slot_changed_seen(&mut self, pos: Pos, items: &[Option<ItemStack>]) {
        let viewers: Vec<(u32, usize)> = self
            .server
            .lan
            .as_ref()
            .map(|host| {
                host.hub
                    .guests
                    .block_mod_menu_viewers(pos)
                    .iter()
                    .map(|&gi| &host.hub.guests[gi])
                    .filter(|g| g.placed && host.hub.here(g))
                    .filter_map(|g| {
                        let open = g.menu.as_ref()?;
                        let layout = open.mod_type?;
                        (open.slots == Slots::Block(pos)).then_some((g.conn, layout.slots))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !self.common.transactions.is_empty() {
            for &(conn, _) in &viewers {
                self.journal_menu_click(conn);
            }
        }
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        for (conn, slots) in viewers {
            if let Some(gi) = host.hub.guests.connection_index(conn) {
                let seen = resized_items(items, slots);
                host.hub.guests.update(gi, |g| {
                    if let Some(open) = g.menu.as_mut()
                        && open.slots == Slots::Block(pos)
                    {
                        open.slot_changed_seen = Some(seen);
                    }
                });
            }
        }
    }

    /// `menu_slot_changed` for a blockless owned menu whose transient slots a committed
    /// transaction changed (`ItemPlace::MenuSlot`). The committed write stands: the listener's
    /// outputs share a transaction of their own, which a failure aborts.
    pub(in crate::game::server_side) fn notify_owned_menu_slot_changed(
        &mut self,
        generation: u64,
        fallback_before: &[Option<ItemStack>],
    ) {
        if self
            .server
            .slot_changed_active
            .contains(&SlotChangedTarget::Owned(generation))
        {
            return;
        }
        let found = self.server.lan.as_ref().and_then(|host| {
            let g = host
                .hub
                .guests
                .iter()
                .find(|g| g.placed && g.menu_generation == generation)?;
            let open = g.menu.as_ref()?;
            let layout = open.mod_type.filter(|t| t.owned_slots.is_some())?;
            if open.slots != Slots::Native
                || !mod_api::__private::menu_listens(mod_api::HookId::MenuSlotChanged, layout.id)
            {
                return None;
            }
            let before = open
                .slot_changed_seen
                .clone()
                .unwrap_or_else(|| resized_items(fallback_before, layout.slots));
            let menu = SlotChangedMenu {
                conn: g.conn,
                uuid: g.uuid,
                window: open.window,
                layout,
                backing: SlotChangedBacking::Owned { generation },
            };
            Some((menu, before, resized_items(&g.native_menu, layout.slots)))
        });
        let Some((menu, before, after)) = found else {
            return;
        };
        self.notify_slot_changed(&menu, &before, &after);
    }

    /// `menu_slot_changed` for the block-backed mod menus open on `pos` after a committed or
    /// external write to its block entity, reported once for all their viewers. A position
    /// nobody has a listening menu open on costs one lookup in the open-menu index: no container
    /// snapshot, no `fallback_before`, no menu context. `fallback_before` gives what the block
    /// held before a committed transaction, for viewers with nothing reported yet.
    pub(in crate::game::server_side) fn notify_block_menu_slot_changed(
        &mut self,
        pos: Pos,
        fallback_before: impl FnOnce(&Self) -> Option<Vec<Option<ItemStack>>>,
    ) {
        if self
            .server
            .slot_changed_active
            .contains(&SlotChangedTarget::Block(pos))
            || !mod_api::__private::has_part_hook(mod_api::HookId::MenuSlotChanged)
        {
            return;
        }
        let viewers = self.server.lan.as_ref().map_or_else(Vec::new, |host| {
            listening_block_menu_viewers(&host.hub.guests, |g| host.hub.here(g), pos)
        });
        let Some(rep) = viewers
            .iter()
            .find(|v| Some(v.conn) == self.server.lan_acting)
            .or(viewers.first())
        else {
            return;
        };
        let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get(&pos) else {
            return;
        };
        #[cfg(any(test, feature = "test-support"))]
        probe::snapshot();
        let after = resized_items(&entity.items, rep.layout.slots);
        let before = match &rep.seen {
            Some(seen) => seen.clone(),
            None => match fallback_before(self) {
                Some(fallback) => resized_items(&fallback, rep.layout.slots),
                None if !rep.sent.is_empty() => resized_items(&rep.sent, rep.layout.slots),
                None => {
                    self.record_block_slot_changed_seen(pos, &after);
                    return;
                }
            },
        };
        let menu = SlotChangedMenu {
            conn: rep.conn,
            uuid: rep.uuid,
            window: rep.window,
            layout: rep.layout,
            backing: SlotChangedBacking::Block {
                pos,
                generation: self.common.world.block_entities.generation(&pos),
            },
        };
        self.notify_slot_changed(&menu, &before, &after);
    }

    /// Report a committed or external write: the listener's outputs commit in a transaction of
    /// their own (the write itself stands), and whatever the menu then holds is its last reported
    /// snapshot even if the listener failed, so the same change is not reported twice.
    fn notify_slot_changed(
        &mut self,
        menu: &SlotChangedMenu,
        before: &[Option<ItemStack>],
        after: &[Option<ItemStack>],
    ) {
        let snapshots = SlotSnapshots { before, after };
        let _ = self.storage_transaction(|game, _| {
            game.mod_menu_slot_changed(menu, None, snapshots)
                .then_some(())
                .ok_or(())
        });
        self.record_slot_changed_seen(menu);
    }
}

struct BlockMenuViewer {
    conn: u32,
    uuid: u128,
    window: i32,
    layout: &'static ModMenuType,
    seen: Option<Vec<Option<ItemStack>>>,
    sent: Vec<Option<ItemStack>>,
}

/// The placed guests passing `here` with a block-backed mod menu open at `pos` whose menu type
/// listens to `menu_slot_changed`, found through the open-menu index: a position nobody views
/// returns at once, without scanning the guest list or allocating.
fn listening_block_menu_viewers(
    guests: &GuestList,
    here: impl Fn(&Guest) -> bool,
    pos: Pos,
) -> Vec<BlockMenuViewer> {
    let mut viewers = Vec::new();
    for &gi in guests.block_mod_menu_viewers(pos) {
        let g = &guests[gi];
        let Some(open) = g.menu.as_ref().filter(|_| g.placed && here(g)) else {
            continue;
        };
        let Some(layout) = open.mod_type else {
            continue;
        };
        if open.slots != Slots::Block(pos)
            || !mod_api::__private::menu_listens(mod_api::HookId::MenuSlotChanged, layout.id)
        {
            continue;
        }
        viewers.push(BlockMenuViewer {
            conn: g.conn,
            uuid: g.uuid,
            window: open.window,
            layout,
            seen: open.slot_changed_seen.clone(),
            sent: open.sent.clone(),
        });
    }
    viewers
}

/// Whether any guest has a block-backed mod menu open at `pos`: the one index lookup a write to
/// an unviewed block costs before `notify_block_menu_slot_changed`.
pub(in crate::game::server_side) fn block_menu_viewed(guests: &GuestList, pos: Pos) -> bool {
    !guests.block_mod_menu_viewers(pos).is_empty()
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

/// What the block-backed fast path costs, counted on the production path for the dailies: the
/// container snapshots taken and the menu contexts built for `menu_slot_changed`.
#[cfg(any(test, feature = "test-support"))]
pub mod probe {
    use std::cell::Cell;

    thread_local! {
        static SNAPSHOTS: Cell<usize> = const { Cell::new(0) };
        static CONTEXTS: Cell<usize> = const { Cell::new(0) };
    }

    pub(in crate::game::server_side) fn snapshot() {
        SNAPSHOTS.set(SNAPSHOTS.get() + 1);
    }

    pub(super) fn context() {
        CONTEXTS.set(CONTEXTS.get() + 1);
    }

    /// `(snapshots, contexts)` since the last call, which starts the count again.
    pub fn take() -> (usize, usize) {
        (SNAPSHOTS.replace(0), CONTEXTS.replace(0))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SlotSnapshots, SlotWriteRefusal, UnreportableSlot, WriteScope, collect_slot_changes,
        context_from_menu, validate_slot_writes,
    };
    use mod_api::{MenuContainer, MenuSlotWrite};
    use sunrise_gameplay_common::game::api_values::StackReadError;
    use sunrise_items::items::{ItemStack, Items};
    use sunrise_level::containers::{ModCrafting, ModLinks, ModMenuType};
    use sunrise_sim::menu::{Kind, Menu};

    fn with_test_item_keys(f: impl FnOnce(mod_api::ItemKey, mod_api::ItemKey, Items)) {
        let mods = mod_api::Mods::default();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::resolve_items([
                "minecraft:iron_sword",
                "minecraft:stick",
                "minecraft:diamond_sword",
                "minecraft:netherite_chestplate",
                "minecraft:diamond_chestplate",
                "minecraft:potion",
            ])
            .unwrap();
            let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
            let potion = mod_api::__private::item_key("minecraft:potion").unwrap();
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "sunrise-slot-changes-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join("items.json"),
                r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
            )
            .unwrap();
            let defaults = sunrise_world::item_defaults::read(&root).unwrap();
            let items =
                Items::catalog(&root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap();
            std::fs::remove_dir_all(root).unwrap();
            f(stick, potion, items);
        });
    }

    #[test]
    fn collect_slot_changes_skips_results_and_linked_slots() {
        with_test_item_keys(|stick, _, _| {
            let layout = ModMenuType {
                id: "test:machine",
                owner: "test",
                slots: 4,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: vec![2],
                inputs: vec![0, 1],
                crafting: None,
                links: Some(ModLinks { from: 3, count: 1 }),
            };
            let before = vec![None, Some(ItemStack::new("minecraft:stick", 1)), None, None];
            let after = vec![
                Some(ItemStack::new("minecraft:stick", 2)),
                Some(ItemStack::new("minecraft:stick", 1)),
                Some(ItemStack::new("minecraft:potion", 1)),
                Some(ItemStack::new("minecraft:potion", 4)),
            ];
            let snapshots = SlotSnapshots {
                before: &before,
                after: &after,
            };
            let changes = collect_slot_changes(&layout, snapshots).unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].container, MenuContainer::Main);
            assert_eq!(changes[0].slot, 0);
            assert_eq!(changes[0].before, None);
            assert_eq!(
                changes[0].after,
                Some(mod_api::ItemStack::new(stick, 2).unwrap())
            );
        });
    }

    #[test]
    fn collect_slot_changes_batches_multi_slot_edits_in_menu_order() {
        with_test_item_keys(|stick, potion, _| {
            let layout = ModMenuType {
                id: "test:processor",
                owner: "test",
                slots: 4,
                player_inventory: true,
                data_slots: 0,
                owned_slots: Some(vec![64, 64, 64, 16]),
                results: vec![3],
                inputs: vec![0, 1, 2],
                crafting: None,
                links: None,
            };
            let before = vec![
                Some(ItemStack::new("minecraft:stick", 1)),
                None,
                Some(ItemStack::new("minecraft:potion", 4)),
                None,
            ];
            let after = vec![
                Some(ItemStack::new("minecraft:stick", 3)),
                Some(ItemStack::new("minecraft:stick", 2)),
                None,
                Some(ItemStack::new("minecraft:potion", 1)),
            ];
            let snapshots = SlotSnapshots {
                before: &before,
                after: &after,
            };
            let changes = collect_slot_changes(&layout, snapshots).unwrap();
            assert_eq!(changes.len(), 3);
            assert_eq!(
                changes.iter().map(|c| c.slot).collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert_eq!(
                changes[0].before,
                Some(mod_api::ItemStack::new(stick, 1).unwrap())
            );
            assert_eq!(
                changes[0].after,
                Some(mod_api::ItemStack::new(stick, 3).unwrap())
            );
            assert_eq!(changes[1].before, None);
            assert_eq!(
                changes[1].after,
                Some(mod_api::ItemStack::new(stick, 2).unwrap())
            );
            assert_eq!(
                changes[2].before,
                Some(mod_api::ItemStack::new(potion, 4).unwrap())
            );
            assert_eq!(changes[2].after, None);
        });
    }

    #[test]
    fn validate_slot_writes_enforces_scope_container_and_slot_caps() {
        with_test_item_keys(|stick, _, items| {
            let owned_layout = ModMenuType {
                id: "test:owned",
                owner: "test",
                slots: 3,
                player_inventory: true,
                data_slots: 0,
                owned_slots: Some(vec![16, 16, 4]),
                results: vec![2],
                inputs: vec![0, 1],
                crafting: None,
                links: None,
            };
            let block_layout = ModMenuType {
                id: "test:block",
                owner: "test",
                slots: 4,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: vec![2],
                inputs: vec![0, 1],
                crafting: None,
                links: Some(ModLinks { from: 3, count: 1 }),
            };
            let four = mod_api::ItemStack::new(stick, 4).unwrap();
            let five = mod_api::ItemStack::new(stick, 5).unwrap();

            // Owned menu accepts declared slot within its per-slot cap and clearing.
            let ok = validate_slot_writes(
                &items,
                |_| 64,
                &owned_layout,
                WriteScope::Owned(owned_layout.owned_slots.as_deref().unwrap()),
                vec![
                    MenuSlotWrite::new(MenuContainer::Main, 2, Some(four.clone())),
                    MenuSlotWrite::new(MenuContainer::Main, 0, None),
                ],
            )
            .unwrap();
            assert_eq!(ok.len(), 2);

            // Over owned slot cap -> refused.
            assert_eq!(
                validate_slot_writes(
                    &items,
                    |_| 64,
                    &owned_layout,
                    WriteScope::Owned(owned_layout.owned_slots.as_deref().unwrap()),
                    vec![MenuSlotWrite::new(MenuContainer::Main, 2, Some(five))],
                ),
                Err(SlotWriteRefusal::StackLimitExceeded)
            );

            // Non-Main container -> refused.
            assert_eq!(
                validate_slot_writes(
                    &items,
                    |_| 64,
                    &owned_layout,
                    WriteScope::Owned(owned_layout.owned_slots.as_deref().unwrap()),
                    vec![MenuSlotWrite::new(
                        MenuContainer::Player,
                        0,
                        Some(four.clone())
                    )],
                ),
                Err(SlotWriteRefusal::NonMainContainer)
            );

            // Block-backed menu accepts result slot, refuses non-result input slot, refuses linked.
            assert!(
                validate_slot_writes(
                    &items,
                    |_| 64,
                    &block_layout,
                    WriteScope::Block(Some(64)),
                    vec![MenuSlotWrite::new(
                        MenuContainer::Main,
                        2,
                        Some(four.clone())
                    )],
                )
                .is_ok()
            );
            assert_eq!(
                validate_slot_writes(
                    &items,
                    |_| 64,
                    &block_layout,
                    WriteScope::Block(Some(64)),
                    vec![MenuSlotWrite::new(
                        MenuContainer::Main,
                        0,
                        Some(four.clone())
                    )],
                ),
                Err(SlotWriteRefusal::NonResultBlockSlot)
            );
            assert_eq!(
                validate_slot_writes(
                    &items,
                    |_| 64,
                    &block_layout,
                    WriteScope::Block(Some(64)),
                    vec![MenuSlotWrite::new(MenuContainer::Main, 3, Some(four))],
                ),
                Err(SlotWriteRefusal::UnavailableSlot)
            );
        });
    }

    #[test]
    fn context_from_menu_includes_main_grid_and_result_containers() {
        with_test_item_keys(|stick, potion, _| {
            let layout: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
                id: "test:crafter",
                owner: "test",
                slots: 6,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: Vec::new(),
                inputs: vec![0, 5],
                crafting: Some(ModCrafting {
                    grid: 0,
                    size: 2,
                    result: 4,
                }),
                links: None,
            }));
            let mut menu = Menu::new(Kind::Mod(layout));
            menu.load_slot(0, Some(ItemStack::new("minecraft:stick", 2)));
            menu.load_slot(4, Some(ItemStack::new("minecraft:potion", 1)));
            menu.load_slot(5, Some(ItemStack::new("minecraft:stick", 3)));

            let snapshot = context_from_menu(&menu, 42, Some([1, 64, 1])).unwrap();
            snapshot.with_context(|ctx| {
                assert_eq!(ctx.player, Some(42));
                assert_eq!(ctx.pos, Some([1, 64, 1]));
                assert_eq!(
                    ctx.slot(MenuContainer::Grid, 0),
                    Some(mod_api::ItemStack::new(stick, 2).unwrap())
                );
                assert_eq!(
                    ctx.slot(MenuContainer::Result, 0),
                    Some(mod_api::ItemStack::new(potion, 1).unwrap())
                );
                assert_eq!(
                    ctx.slot(MenuContainer::Main, 5),
                    Some(mod_api::ItemStack::new(stick, 3).unwrap())
                );
                assert_eq!(ctx.slot(MenuContainer::Main, 0), None);
                assert_eq!(ctx.slot(MenuContainer::Main, 4), None);
            });
        });
    }

    #[test]
    fn a_changed_stack_the_api_cant_carry_refuses_the_batch_and_a_count_0_stack_is_empty() {
        with_test_item_keys(|_, _, _| {
            let layout = ModMenuType {
                id: "test:processor",
                owner: "test",
                slots: 3,
                player_inventory: true,
                data_slots: 0,
                owned_slots: Some(vec![64, 64, 64]),
                results: vec![2],
                inputs: vec![0, 1],
                crafting: None,
                links: None,
            };
            // `minecraft:apple` has no API key in this fixture (`with_test_item_keys`).
            let before = vec![None, Some(ItemStack::new("minecraft:stick", 0)), None];
            let after = vec![Some(ItemStack::new("minecraft:apple", 1)), None, None];
            let snapshots = SlotSnapshots {
                before: &before,
                after: &after,
            };
            assert_eq!(
                collect_slot_changes(&layout, snapshots),
                Err(UnreportableSlot {
                    container: MenuContainer::Main,
                    slot: 0,
                    reason: StackReadError::UnknownItem,
                })
            );
            // A count-0 stack left in slot 1 is empty: emptying it changes nothing.
            let after = vec![None, None, None];
            let snapshots = SlotSnapshots {
                before: &before,
                after: &after,
            };
            assert_eq!(collect_slot_changes(&layout, snapshots), Ok(Vec::new()));
        });
    }

    #[test]
    fn listening_block_viewers_are_the_placed_guests_here_with_a_listening_menu_open_there() {
        use std::sync::Arc;
        use sunrise_sim::lan::{
            Joined, PlayerSender,
            guest::Guest,
            guest_list::GuestList,
            menu::{Menu as OpenMenu, Slots},
        };

        const LISTENING_MENU: &str = "test:listening_machine";
        const SILENT_MENU: &str = "test:silent_machine";

        struct ListeningPart;
        #[mod_api::__subscriptions]
        impl mod_api::MenuPart for ListeningPart {
            fn menu_slot_changed(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mut mod_api::MenuSlotChangedEvent<'_>,
            ) {
            }
        }
        struct TestMod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for TestMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.menu_part(LISTENING_MENU, ListeningPart);
            }
        }

        let make_guest = |conn: u32, placed: bool, dim: &str| {
            let (tx, _) = std::sync::mpsc::channel();
            let mut g = Guest::new(
                Joined {
                    conn,
                    uuid: u128::from(conn) + 500,
                    name: format!("guest{conn}"),
                    properties: Vec::new(),
                    info: sunrise_protocol::packets::configuration::ClientInformation::new(8),
                    brand: "sunrise".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                conn as i32 + 20,
                [0.0, 64.0, 0.0],
                8,
            );
            g.placed = placed;
            g.dimension = dim.to_owned();
            g
        };
        let layout = |id: &'static str| -> &'static ModMenuType {
            Box::leak(Box::new(ModMenuType {
                id,
                owner: "test",
                slots: 2,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: vec![1],
                inputs: vec![0],
                crafting: None,
                links: None,
            }))
        };
        let (listening, silent) = (layout(LISTENING_MENU), layout(SILENT_MENU));
        let open_at = |layout: &'static ModMenuType, pos: super::Pos| OpenMenu {
            window: 1,
            kind: 0,
            registry_id: layout.id.into(),
            pos: Some(pos),
            slots: Slots::Block(pos),
            size: layout.slots,
            state: 0,
            sent: Vec::new(),
            drag: None,
            sent_data: Vec::new(),
            reported_unsent_data_len: None,
            blockless_data: Vec::new(),
            mod_type: Some(layout),
            craft: None,
            links: Vec::new(),
            asks_still_valid: false,
            opener_block: None,
            opening_item: None,
            removed: false,
            slot_changed_seen: None,
        };

        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("test", Box::new(TestMod))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            let (target, other) = ((10, 64, 10), (20, 64, 20));
            let viewers = |guests: &GuestList| {
                super::listening_block_menu_viewers(
                    guests,
                    |g| g.placed_in("minecraft:overworld"),
                    target,
                )
                .iter()
                .map(|v| v.conn)
                .collect::<Vec<_>>()
            };
            let mut guests = GuestList::default();
            // Nobody counts: a viewer of another block, an unplaced guest, a guest in another
            // dimension and a viewer of a menu type no part listens on.
            guests.push(make_guest(1, true, "minecraft:overworld"));
            guests.push(make_guest(2, false, "minecraft:overworld"));
            guests.push(make_guest(3, true, "minecraft:the_nether"));
            guests.push(make_guest(4, true, "minecraft:overworld"));
            guests.update(0, |g| g.menu = Some(open_at(listening, other)));
            guests.update(1, |g| g.menu = Some(open_at(listening, target)));
            guests.update(2, |g| g.menu = Some(open_at(listening, target)));
            guests.update(3, |g| g.menu = Some(open_at(silent, target)));
            // The index finds the target's open menus; the filters leave none of them.
            assert!(super::block_menu_viewed(&guests, target));
            assert!(viewers(&guests).is_empty());
            // A placed guest here with the listening menu open at the target is the one viewer.
            guests.update(0, |g| g.menu = Some(open_at(listening, target)));
            assert_eq!(viewers(&guests), [1]);
            // Closing the menu or leaving takes it out of the index again.
            guests.update(0, |g| g.menu = None);
            assert!(viewers(&guests).is_empty());
            guests.update(0, |g| g.menu = Some(open_at(listening, target)));
            guests.remove(0);
            assert!(viewers(&guests).is_empty());
            // A block nobody has a menu open on costs this one lookup.
            assert!(!super::block_menu_viewed(&guests, other));
        });
    }
}
