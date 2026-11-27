//! A player's break for native mods (`PlayerRef::destroy_block`): what a vein miner needs to
//! break the blocks around the one its player broke as that player breaks them, taking their
//! drops and experience in `block_drops`.
use crate::game::ServerGame;
use mod_api::{BlockFace, Placement, PlayerRef};
use std::cell::Cell;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
use sunrise_gameplay_common::game::native_hooks::player_destroyed_block::GameHooks as _;
use sunrise_gameplay_common::game::native_world_read_common::block_face_direction;
use sunrise_sim::first_person_sim::Hand;
use sunrise_world::registry::BlockStateId;

/// How many veto asks may be inside one another on the server thread. A `break_block` or
/// `entity_place` listener may ask again as the player without changing the world: a listener
/// with no guard of its own would recurse until the stack overflows. At the limit the call is
/// refused. A Sunrise safeguard: NeoForge has none, and there the same listener crashes the
/// server.
const MAX_NESTED_BREAKS: u32 = 8;

thread_local! {
    /// The veto asks now inside one another on this thread.
    static NESTED_BREAKS: Cell<u32> = const { Cell::new(0) };
}

/// One level of [`NESTED_BREAKS`], given back when dropped, a panic unwinding included.
struct NestedBreak;

impl NestedBreak {
    /// A level, or none at [`MAX_NESTED_BREAKS`].
    fn enter() -> Option<Self> {
        NESTED_BREAKS.with(|depth| {
            (depth.get() < MAX_NESTED_BREAKS).then(|| {
                depth.set(depth.get() + 1);
                Self
            })
        })
    }
}

impl Drop for NestedBreak {
    fn drop(&mut self) {
        NESTED_BREAKS.with(|depth| depth.set(depth.get() - 1));
    }
}

impl ServerGame<'_> {
    /// `ServerPlayerGameMode.destroyBlock` at `pos` as `player` breaks it, as FTB Ultimine does
    /// (`PlayerInteractionBackendMut::player_destroy_block` says what holds and what is refused). With
    /// `into_inventory`, the drops `block_drops` leaves go to the player's inventory, and what
    /// doesn't fit drops from them as `Inventory.placeItemBackInInventory` drops it. Refused
    /// (false) once [`MAX_NESTED_BREAKS`] breaks are inside one another.
    pub fn player_destroy_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        into_inventory: bool,
    ) -> bool {
        if !self.native_authoritative() {
            return false;
        }
        let mut broke = false;
        self.native_as_player_handle(player, |game| {
            if !game.acting_in_level_in_place() {
                return;
            }
            // Only this break owns these drops. Other breaks, including ones made by
            // callbacks, cannot inherit this destination, even if this call unwinds.
            let mut drops = into_inventory.then(Vec::new);
            let result = game.with_hand(Hand::Main, |game| {
                game.player_break_block(pos, drops.as_mut())
            });
            for stack in drops.into_iter().flatten() {
                game.give_or_drop(stack);
            }
            broke = result.unwrap_or_else(|e| {
                log::warn!("a mod's break at {pos:?} failed: {e}");
                false
            });
        });
        broke
    }

    /// `CommonHooks.fireBlockBreak` for `player` at `pos`, breaking nothing
    /// (`PlayerInteractionBackendMut::player_break_canceled` says what is refused).
    pub fn player_break_canceled(&mut self, player: PlayerRef, pos: [i32; 3]) -> Option<bool> {
        if !self.native_authoritative() || !self.native_player_current(player) {
            return None;
        }
        let _nested = NestedBreak::enter()?;
        let mut canceled = None;
        self.native_as_player_handle(player, |game| {
            if game.acting_in_level_in_place() && game.world.loaded(pos[0], pos[2]) {
                canceled = game
                    .break_block_allowed(pos)
                    .inspect_err(|e| log::warn!("a mod's break ask at {pos:?} failed: {e}"))
                    .ok()
                    .map(|allowed| allowed.is_none());
            }
        });
        canceled
    }

    /// `EventHooks.onBlockPlace` for `player` placing `state` at `pos` against its clicked
    /// `face`, placing nothing (`PlayerInteractionBackendMut::player_place_canceled` says what is
    /// refused).
    pub fn player_place_canceled(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        face: BlockFace,
        state: BlockStateId,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        if !self.native_authoritative() || !self.native_player_current(player) {
            return None;
        }
        let _nested = NestedBreak::enter()?;
        // The clicked block, `pos.relative(face.getOpposite())`.
        let normal = block_face_direction(face).normal();
        let against = [0, 1, 2].map(|axis| pos[axis].wrapping_sub(normal[axis]));
        let mut canceled = None;
        self.native_as_player_handle(player, |game| {
            if game.acting_in_level_in_place()
                && game.world.loaded(pos[0], pos[2])
                && game.world.loaded(against[0], against[2])
            {
                let placed_against = game.world.block(against[0], against[1], against[2]);
                let mut placing = false;
                game.native_entity_place(
                    Some(player.entity()),
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    state,
                    placed_against,
                    Placement::Single,
                    &mut placing,
                );
                canceled = Some(placing);
            }
        });
        canceled
    }
}

impl ServerGame<'_> {
    /// `ServerPlayerGameMode.destroyBlock`'s `Level.removeBlock`, then `Block.destroy`: the mods'
    /// `player_destroyed_block` hears the removal once it succeeded, before a creative player's
    /// return and the drops. It is told the block the removal took out, read here, after
    /// `playerWillDestroy` and the mods' `break_block` had their say, together with that block's
    /// block entity data. True when the block was removed.
    pub(in crate::game::server_side) fn remove_block_as_player(
        &mut self,
        pos: [i32; 3],
        creative: bool,
        correct_tool: bool,
    ) -> bool {
        let source = self
            .server
            .lan_acting_uuid
            .map(sunrise_world::level::LevelEventPlayer);
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let block_entity = self.removed_block_entity_data(pos, state);
        let removed = self.with_level(|b, level, ticks| {
            b.player_destroy_block(
                level,
                ticks,
                sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                creative,
                correct_tool,
                source,
            )
        });
        if removed {
            self.invalidate_active_player_breaks(pos);
            self.native_player_destroyed_block(pos, state, block_entity);
        }
        removed
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
