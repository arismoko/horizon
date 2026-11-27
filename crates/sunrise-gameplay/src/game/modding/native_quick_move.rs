//! A shift-click in a mod container's menu, routed by the mods
//! (`native_menu_quick_move`, `AbstractContainerMenu.quickMoveStack`) before
//! the game's own routing.
use super::BlockStateId;
use super::slot_changes::{SlotChangedBacking, SlotChangedMenu, SlotSnapshots};
use crate::game::server_side::native_hooks_server::block_entity_tick::ResidentBlockEntityAccess;
use mod_api::{BlockEntityRef, BlockEntityScope, BlockEntityTypeKey};
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
use sunrise_gameplay_common::game::transactions::After;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    block_entities::Pos,
    containers::{BlockEntity, ModCrafting, ModMenuType},
};
use sunrise_protocol::packets::play;
use sunrise_sim::{
    lan::menu::{ModCraft, Slots},
    menu::Menu,
};

/// `ClickType.QUICK_MOVE`'s id in `ServerboundContainerClickPacket`.
const QUICK_MOVE: i32 = 1;

/// Where a menu slot of a block container's menu lives: the container's own
/// slot `k`, or the player's inventory index (hotbar 0 to 8, then the 27
/// main slots), the menu listing the main slots before the hotbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum MenuSlot {
    Container(usize),
    Inventory(usize),
}

/// The slot a quick move of `slot` on a menu with `size` container slots
/// asks a mod about, or None when the click is not one: another click type,
/// a button other than left or right (`doClick` takes 0 and 1), no slot,
/// or a slot past the menu.
pub(in crate::game) fn quick_move_slot(
    input: i32,
    button: i32,
    slot: i32,
    size: usize,
) -> Option<MenuSlot> {
    if input != QUICK_MOVE || !(0..=1).contains(&button) {
        return None;
    }
    let slot = usize::try_from(slot).ok()?;
    if slot < size {
        return Some(MenuSlot::Container(slot));
    }
    let n = slot - size;
    match n {
        0..27 => Some(MenuSlot::Inventory(9 + n)),
        27..36 => Some(MenuSlot::Inventory(n - 27)),
        _ => None,
    }
}

/// Storage behind the clicked slot in the admitted menu.
#[derive(Clone, Copy)]
enum Source {
    Own(usize),
    Player(usize),
    Linked {
        pos: Pos,
        slot: usize,
        generation: u64,
    },
    CraftingResult(ModCrafting),
}

struct QuickMove {
    slot: usize,
    source: Source,
    held: ItemStack,
}

impl crate::game::ServerGame<'_> {
    /// `ServerGamePacketListenerImpl.handleContainerClick` for a quick move
    /// in a guest's open mod container menu (on a vanilla menu type or the
    /// mod's own): the mods' `native_menu_quick_move` first, as the clicking player.
    /// A mod menu type supplies the live menu after `Slot.mayPickup`. `None` lets ordinary
    /// routing continue; `Some(false)` is a refusal, not a second route.
    pub(in crate::game::server_side) fn lan_mod_quick_move(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        live: Option<&Menu>,
    ) -> Option<bool> {
        use sunrise_gameplay_common::game::lan_menu_data::ClickReply;
        if p.input != QUICK_MOVE
            || !mod_api::__private::has_part_hook(mod_api::HookId::MenuQuickMove)
        {
            return None;
        }
        let host = self.server.lan.as_ref()?;
        let g = host.hub.guest(conn)?;
        let menu = g.menu.as_ref().filter(|m| {
            p.container != 0
                && m.window == p.container
                && m.mod_type.is_some() == live.is_some()
                && usize::try_from(p.slot).is_ok_and(|slot| slot < m.slot_count())
        })?;
        let Slots::Block(pos) = menu.slots else {
            return None;
        };
        let at = quick_move_slot(p.input, i32::from(p.button), i32::from(p.slot), menu.size)?;
        // A shift-click during a drag only ends the drag (`doClick`'s resetQuickCraft).
        // The ordinary path also answers spectators and dead players.
        if g.spectator() || g.dead || menu.drag.is_some() {
            return None;
        }
        let (size, window, menu_type) = (menu.size, menu.window, menu.mod_type);
        let generation = g.menu_generation;
        let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get(&pos) else {
            return None;
        };
        if !mod_api::__private::block_entity_type_listens(
            mod_api::HookId::MenuQuickMove,
            entity.id(),
        ) {
            return None;
        }
        let kind = mod_api::__private::block_entity_type_key(entity.id())?;
        let (subject, state) = self.resident_block_entity_target(pos)?;
        let slot = p.slot as usize;
        let held = match (live, at) {
            (Some(menu), _) => menu.slot(slot).cloned(),
            (None, MenuSlot::Container(k)) => entity.items.get(k).cloned().flatten(),
            (None, MenuSlot::Inventory(i)) => g.inventory[i].clone(),
        };
        let held = held.filter(|s| s.count > 0)?;
        let source = if let Some(live) = live
            && let Some(craft) = live.mod_crafting().filter(|craft| craft.result == slot)
        {
            Source::CraftingResult(craft)
        } else if menu_type
            .and_then(|t| t.links)
            .is_some_and(|link| link.slots().contains(&slot))
        {
            let (pos, slot) = menu.linked_slots(&self.common.world)?.backing(slot)??;
            Source::Linked {
                pos,
                slot,
                generation: self.common.world.block_entities.generation(&pos)?,
            }
        } else {
            match at {
                MenuSlot::Container(k) => Source::Own(k),
                MenuSlot::Inventory(i) => Source::Player(i),
            }
        };
        let quick_move = QuickMove { slot, source, held };
        let admitted = Admitted::of(self, conn, pos, size, window)?;
        let outcome = self.as_guest(conn, |game| {
            game.run_mod_quick_move(&admitted, menu_type, kind, (subject, state), quick_move)
        });
        if !self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .is_some_and(|g| g.menu_generation == generation)
        {
            // The callback already replaced or closed this menu.
            return Some(false);
        }
        if self.native_block_entity_write_key(subject).is_none() {
            self.reply_menu_click(conn, ClickReply::Close(window));
            return Some(true);
        }
        match outcome {
            Some(QuickMoveOutcome::Handled) => {}
            Some(QuickMoveOutcome::Untaken) => return None,
            Some(QuickMoveOutcome::Rejected) | None => {
                self.reply_menu_click(conn, ClickReply::Resend(window));
                return Some(false);
            }
        }
        // The mod menu type's engine starts over from the block entity next click.
        if let Some(play) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            && play
                .crafting
                .machine
                .as_ref()
                .is_some_and(|(w, _)| *w == window)
        {
            *play.crafting.machine = None;
        }
        // sendAllDataToRemote: the client predicted the game's routing.
        self.reply_menu_click(conn, ClickReply::Resend(window));
        Some(true)
    }

    fn run_mod_quick_move(
        &mut self,
        admitted: &Admitted,
        take_menu: Option<&'static ModMenuType>,
        kind: BlockEntityTypeKey,
        (subject, state): (BlockEntityRef, BlockStateId),
        quick_move: QuickMove,
    ) -> QuickMoveOutcome {
        let Admitted {
            conn,
            window,
            pos,
            size,
            ..
        } = *admitted;
        let QuickMove {
            slot,
            source,
            held: clicked,
        } = quick_move;
        let live = |game: &Self| {
            let Some(open) = admitted.menu(game) else {
                return false;
            };
            if game.native_block_entity_write_key(subject) != Some(pos) {
                return false;
            }
            match source {
                Source::Linked {
                    pos: link_pos,
                    slot: link_slot,
                    generation,
                } => {
                    game.world.block_entities.generation(&link_pos) == Some(generation)
                        && open
                            .linked_slots(&game.world)
                            .and_then(|links| links.backing(slot))
                            .flatten()
                            == Some((link_pos, link_slot))
                }
                _ => true,
            }
        };
        if !live(self) {
            return QuickMoveOutcome::Rejected;
        }
        let mut held = match Some(clicked.clone()).try_into_api() {
            Ok(held) => held,
            Err(reason) => {
                log::warn!(
                    "menu_quick_move: cannot project the clicked stack at {pos:?} slot {slot}: {reason:?}"
                );
                return QuickMoveOutcome::Rejected;
            }
        };
        let original = held.clone();
        // `AbstractContainerMenu.slotsChanged`: a mod menu type listening to `menu_slot_changed`
        // recomputes its outputs from the settled move, inside the same transaction.
        let changed_menu = take_menu.filter(|menu| {
            mod_api::__private::menu_listens(mod_api::HookId::MenuSlotChanged, menu.id)
        });
        let (result, notices) = self.collect_quick_move_notices(|game| {
            game.storage_transaction(|game, _| {
                game.transactions.last_mut().unwrap().menu_click_active = Some(conn);
                // Only a listening menu pays for the snapshot.
                let before = changed_menu.and_then(|_| match game.world.block_entities.get(&pos) {
                    Some(BlockEntity::Mod(entity)) => Some(entity.items.clone()),
                    _ => None,
                });
                let mut handled = false;
                let mut access = ResidentBlockEntityAccess::for_handle(game, subject, state);
                let panicked = sunrise_gameplay_common::game::native_hooks::native_menu_quick_move(
                    kind,
                    BlockEntityScope::new(&mut access),
                    size,
                    slot,
                    &mut held,
                    &mut handled,
                );
                let items_changed = access.items_changed();
                // The destination edits are provisional until the original route and clicked
                // stack still admit the move. Vanilla's moveItemStackTo shrinks the actual
                // source while filling the destination; our hook proposes that shrink separately.
                if !live(game) {
                    return Err(QuickMoveOutcome::Rejected);
                }
                let source_stack = match source {
                    Source::Own(slot) => game
                        .world
                        .block_entities
                        .get(&pos)
                        .and_then(|entity| entity.slots().get(slot))
                        .and_then(Option::as_ref),
                    Source::Player(slot) => game.inventory[slot].as_ref(),
                    Source::Linked { pos, slot, .. } => game
                        .world
                        .block_entities
                        .get(&pos)
                        .and_then(|entity| entity.slots().get(slot))
                        .and_then(Option::as_ref),
                    // A handler may consume crafting inputs; the result below belongs to the
                    // grid it leaves, not to a fresh result computed from that grid.
                    Source::CraftingResult(_) => Some(&clicked),
                };
                if source_stack != Some(&clicked) || (panicked && !handled) {
                    return Err(QuickMoveOutcome::Rejected);
                }
                if !handled {
                    return Err(QuickMoveOutcome::Untaken);
                }
                let remainder = game.project_quick_move_remainder(&clicked, &held, &original)?;
                let take = take_menu
                    .filter(|menu| menu.owns_slot(slot))
                    .and_then(|menu| {
                        let count = clicked
                            .count
                            .saturating_sub(remainder.as_ref().map_or(0, |left| left.count));
                        (count > 0).then(|| {
                            let mut moved = clicked;
                            moved.count = count;
                            (menu.id, (slot, moved, remainder.clone()))
                        })
                    });
                let mut changed = items_changed;
                match source {
                    Source::Own(slot) => {
                        game.journal_block_entity(pos);
                        let dest = game
                            .world
                            .block_entities
                            .get_mut(&pos)
                            .and_then(|entity| entity.slot_mut(slot))
                            .ok_or(QuickMoveOutcome::Rejected)?;
                        *dest = remainder;
                        changed = true;
                    }
                    Source::Player(slot) => {
                        let player = game.native_acting_id();
                        game.journal_player(player);
                        game.inventory[slot] = remainder;
                    }
                    Source::Linked { pos, slot, .. } => {
                        game.journal_block_entity_slot(pos, slot);
                        let dest = game
                            .world
                            .block_entities
                            .get_mut(&pos)
                            .and_then(|entity| entity.slot_mut(slot))
                            .ok_or(QuickMoveOutcome::Rejected)?;
                        if *dest != remainder {
                            *dest = remainder;
                            game.after_commit(After::Changed(pos));
                        }
                    }
                    Source::CraftingResult(layout) => {
                        let gi = game.guest_index(conn).ok_or(QuickMoveOutcome::Rejected)?;
                        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos)
                        else {
                            return Err(QuickMoveOutcome::Rejected);
                        };
                        let grid = layout
                            .grid_slots()
                            .map(|k| entity.items.get(k).cloned().flatten())
                            .collect();
                        let player = game.native_acting_id();
                        if let Some(player) = game.player(player) {
                            game.journal_menus(player);
                        }
                        game.lan_mod_craft_store(
                            gi,
                            window,
                            ModCraft {
                                grid,
                                result: remainder,
                            },
                        );
                    }
                }
                if changed {
                    game.after_commit(After::Changed(pos));
                }
                // The listener sees the move with its source settled, and an invalid output or a
                // failed listener aborts the whole move with this transaction.
                if let Some(layout) = changed_menu
                    && let Some(before) = before
                    && let Some(uuid) = game
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|guest| guest.uuid)
                    && let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos)
                {
                    let after = entity.items.clone();
                    let menu = SlotChangedMenu {
                        conn,
                        uuid,
                        window,
                        layout,
                        backing: SlotChangedBacking::Block {
                            pos,
                            generation: admitted.generation,
                        },
                    };
                    let snapshots = SlotSnapshots {
                        before: &before,
                        after: &after,
                    };
                    if !game.mod_menu_slot_changed(&menu, None, snapshots) {
                        return Err(QuickMoveOutcome::Rejected);
                    }
                }
                Ok(take)
            })
        });
        let take = match result {
            Ok(take) => take,
            Err(outcome) => return outcome,
        };
        self.publish_quick_move_notices(notices);
        if let Some((menu, take)) = take
            && admitted.holds(self)
            && self.native_block_entity_write_key(subject) == Some(pos)
        {
            let player = self.native_acting_id();
            let _ = self.mod_menu_slot_take(Some(pos), player, menu, &take);
        }
        QuickMoveOutcome::Handled
    }

    /// Preserve untouched engine values and validate an edited remainder once.
    fn project_quick_move_remainder(
        &self,
        clicked: &ItemStack,
        held: &Option<mod_api::ItemStack>,
        original: &Option<mod_api::ItemStack>,
    ) -> Result<Option<ItemStack>, QuickMoveOutcome> {
        if held == original {
            Ok(Some(clicked.clone()))
        } else {
            held.try_into_engine(self.items()).map_err(|error| {
                log::warn!("mod menu_quick_move write refused: {error:?}");
                QuickMoveOutcome::Rejected
            })
        }
    }
}

enum QuickMoveOutcome {
    /// A mod listener handled the shift-click and its writes stood.
    Handled,
    /// No mod took the click; continue with the admitted menu's quick-move.
    Untaken,
    /// The proposal was invalid, panicked, or its admitted source went stale.
    Rejected,
}

/// What `lan_mod_quick_move` admitted the click on, read again before the move is applied: the
/// menu, the block entity behind it (a replaced one has another generation though its values may
/// be equal) and the inputs a listener's command could change (the guest's mode and life, a drag).
/// The validity hook and the slot rules are not asked again.
pub struct Admitted {
    conn: u32,
    window: i32,
    pos: Pos,
    size: usize,
    menu_generation: u64,
    generation: Option<u64>,
}

impl Admitted {
    pub fn of(
        game: &crate::game::ServerGame<'_>,
        conn: u32,
        pos: Pos,
        size: usize,
        window: i32,
    ) -> Option<Self> {
        let guest = game.server.lan.as_ref()?.hub.guest(conn)?;
        guest.menu.as_ref()?;
        Some(Self {
            conn,
            window,
            pos,
            size,
            menu_generation: guest.menu_generation,
            generation: game.world.block_entities.generation(&pos),
        })
    }

    /// Whether the same window, guest mode and original backing generation are still admitted.
    /// Callers separately check the level-bound holder before settling the clicked slot.
    pub fn holds(&self, game: &crate::game::ServerGame<'_>) -> bool {
        self.menu(game).is_some()
    }

    fn menu<'a>(
        &self,
        game: &'a crate::game::ServerGame<'_>,
    ) -> Option<&'a sunrise_sim::lan::menu::Menu> {
        let guest = game.server.lan.as_ref()?.hub.guest(self.conn)?;
        let open = guest.menu.as_ref()?;
        (guest.menu_generation == self.menu_generation
            && open.window == self.window
            && open.slots == Slots::Block(self.pos)
            && open.size == self.size
            && open.drag.is_none()
            && !guest.spectator()
            && !guest.dead
            && game
                .world
                .block_entities
                .generation(&self.pos)
                .is_none_or(|now| Some(now) == self.generation))
        .then_some(open)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::{MenuSlot, quick_move_slot};

    /// A quick move asks about the clicked slot only: a `generic_9x1` menu
    /// has the container's 9, then the main inventory, then the hotbar.
    #[test]
    fn a_quick_move_names_the_container_or_inventory_slot() {
        assert_eq!(quick_move_slot(1, 0, 0, 9), Some(MenuSlot::Container(0)));
        assert_eq!(quick_move_slot(1, 1, 8, 9), Some(MenuSlot::Container(8)));
        // The first main slot is inventory index 9, the last 35.
        assert_eq!(quick_move_slot(1, 0, 9, 9), Some(MenuSlot::Inventory(9)));
        assert_eq!(quick_move_slot(1, 0, 35, 9), Some(MenuSlot::Inventory(35)));
        // The hotbar comes last in the menu and first in the inventory.
        assert_eq!(quick_move_slot(1, 0, 36, 9), Some(MenuSlot::Inventory(0)));
        assert_eq!(quick_move_slot(1, 0, 44, 9), Some(MenuSlot::Inventory(8)));
    }

    /// Only a left or right shift-click on a slot the menu has is a quick
    /// move: no other click type, button, outside click or unavailable slot.
    #[test]
    fn other_clicks_are_not_quick_moves() {
        // PICKUP, SWAP, CLONE, THROW, QUICK_CRAFT, PICKUP_ALL.
        for input in [0, 2, 3, 4, 5, 6] {
            assert_eq!(quick_move_slot(input, 0, 0, 9), None);
        }
        assert_eq!(quick_move_slot(1, 2, 0, 9), None);
        assert_eq!(quick_move_slot(1, -1, 0, 9), None);
        assert_eq!(quick_move_slot(1, 0, -999, 9), None);
        assert_eq!(quick_move_slot(1, 0, 45, 9), None);
    }
}
