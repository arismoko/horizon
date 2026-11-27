//! Placement-state reads, placement writes and the `Block.setPlacedBy` step.
use mod_api::{
    BlockFace, BlockKey, BlockUpdateFlags, HookId, LevelRef, LevelWriteError, PlacementStateError,
    PlayerRef, PlayerWorldRefusal as Refusal,
};
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_gameplay_common::game::{
    api_values::block_state::block_listens, native_server::PlayerRead,
    native_world_read_common::block_face_direction,
};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_world::{
    level::{PlacementContext, classes::Class},
    registry::BlockStateId,
};

/// A placement's write flags: neighbours and clients told, shapes updated unless known.
fn placement_flags(update_neighbour_shapes: bool) -> BlockUpdateFlags {
    let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
    if update_neighbour_shapes {
        flags
    } else {
        flags | BlockUpdateFlags::KNOWN_SHAPE
    }
}

impl crate::game::ServerGame<'_> {
    /// The one block write (`level_set_block`, its block entity included) with a placement's
    /// flags, then `Block.setPlacedBy` in the player's turn; nothing else of `BlockItem.place`.
    pub fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        face: BlockFace,
        update_neighbour_shapes: bool,
    ) -> Result<(), Refusal> {
        if !self.common.is_logical_server() {
            return Err(Refusal::Client);
        }
        self.validate_placement_player(player)?;
        if self.common.pack.assets.registry.state(state).is_none() {
            return Err(Refusal::UnknownState);
        }
        if self.native_level_world(level).is_none() {
            return Err(Refusal::StaleLevel);
        }
        if level.dimension() != self.common.dim.key() {
            return Err(Refusal::OtherLevel);
        }
        let [x, y, z] = pos;
        if !self.common.world.shape.contains_y(y)
            || !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
        {
            return Err(Refusal::OutOfWorld { pos });
        }
        if !self.common.world.loaded(x, z) {
            return Err(Refusal::NotLoaded { pos });
        }
        if self.common.world.block(x, y, z) == state {
            return Err(Refusal::Unchanged);
        }
        let flags = placement_flags(update_neighbour_shapes);
        if !self
            .placement_allowed(pos, state, Some(flags))
            .map_err(|_| Refusal::InTransaction)?
        {
            return Err(Refusal::Fenced);
        }
        let listens = mod_api::__private::listened(HookId::EntityPlace);
        if !listens
            && self.common.transactions.is_empty()
            && !block_listens(HookId::SetPlacedBy, state)
        {
            if !self
                .level_set_block(level, pos, state, flags)
                .map_err(|_| Refusal::Stale)?
            {
                return Err(Refusal::Unchanged);
            }
            return self
                .native_as_player_handle(player, |game| {
                    game.run_mod_placement_callback(pos, state, None);
                })
                .then_some(())
                .ok_or(Refusal::Stale);
        }

        self.storage_transaction(|game, _| {
            use super::journal_level_write::JournalLevelWrite;
            game.begin_placement_capture();
            if !JournalLevelWrite::new(game, level)
                .ok_or(Refusal::StaleLevel)?
                .write_block(pos, state, flags.bits())
            {
                return Err(Refusal::Unchanged);
            }
            let mut callback = Ok(());
            if !game.native_as_player_handle(player, |game| {
                callback = game.stage_placement_callback(level, pos, state, None);
            }) {
                return Err(Refusal::Stale);
            }
            callback?;
            if !listens {
                return Ok(());
            }
            let writes = game.placement_footprint();
            let (at, placed) = writes[0];
            let [dx, dy, dz] = block_face_direction(face).opposite().normal();
            let against = game.common.world.block(x + dx, y + dy, z + dz);
            if game.placement_canceled(
                Some(player.entity()),
                at,
                placed,
                against,
                writes[1..].iter().copied(),
            ) {
                return Err(Refusal::Canceled);
            }
            if !game.level_handle_current(level) {
                return Err(Refusal::StaleLevel);
            }
            if level.dimension() != game.common.dim.key() {
                return Err(Refusal::OtherLevel);
            }
            game.validate_placement_player(player)?;
            if writes.into_iter().any(|(at, state)| {
                !game.common.world.is_loaded_at(at.x, at.y, at.z)
                    || game
                        .common
                        .world
                        .carrier_fences
                        .locks_cell(&[at.x, at.y, at.z])
                    || game.common.world.block(at.x, at.y, at.z) != state
            }) {
                return Err(Refusal::PlacementChanged);
            }
            Ok(())
        })
    }

    fn validate_placement_player(&self, player: PlayerRef) -> Result<(), Refusal> {
        let (dead, same_level) = match self.player_handle_read(player).ok_or(Refusal::Stale)? {
            PlayerRead::Acting { body, .. } => (
                body.is_none_or(|p| p.health.health() <= 0.0),
                self.acting_in_level_in_place(),
            ),
            PlayerRead::Guest(guest) => (guest.dead, guest.dimension == self.common.dim.id()),
        };
        if dead {
            Err(Refusal::Dead)
        } else if !same_level {
            Err(Refusal::OtherLevel)
        } else {
            Ok(())
        }
    }

    pub(in crate::game) fn publish_placement_effect(
        &mut self,
        effect: sunrise_world::level::ticks::PlacementEffect,
    ) {
        self.with_level(|b, level, ticks| b.publish_placement_effect(level, ticks, effect));
        let _ = self.server_process_level_events();
    }

    /// Run both pieces of `Block.setPlacedBy` before offering a veto. The simulation
    /// records its ordered writes and effects; the game and Part then read those
    /// writes and use the same savepoint for any further edits.
    fn stage_placement_callback(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        stack: Option<&ItemStack>,
    ) -> Result<(), Refusal> {
        use super::journal_level_write::JournalLevelWrite;
        use sunrise_gameplay_common::game::transactions::After;
        use sunrise_world::level::ticks::PlacementOperation;
        let [x, y, z] = pos;
        let (operations, random) = self.capture_placed_by(BlockPos::new(x, y, z), state);
        if <(u64, Option<f64>)>::from(&random) != (&*self.common.world.random).into() {
            self.journal_set_level_random(random);
        }
        for operation in operations {
            match operation {
                PlacementOperation::Write(cell, state, flags) => {
                    if !JournalLevelWrite::new(self, level)
                        .ok_or(Refusal::StaleLevel)?
                        .write_block([cell.x, cell.y, cell.z], state, flags)
                    {
                        return Err(Refusal::PlacementChanged);
                    }
                }
                PlacementOperation::Effect(effect) => {
                    self.after_commit(After::PlacementEffect(Box::new(effect)));
                }
            }
        }
        self.set_placed_by(pos, None, stack)
            .map_err(|_| Refusal::PlacementChanged)
    }
    pub(in crate::game) fn level_state_for_placement(
        &mut self,
        level: LevelRef,
        block: BlockKey,
        context: &mod_api::PlacementContext,
    ) -> Result<Option<BlockStateId>, PlacementStateError> {
        let world = self.native_level_world_result(level)?;
        if level.dimension() != self.common.dim.key() {
            return Err(PlacementStateError::Unavailable);
        }
        let BlockPos { x, y, z } = context.pos;
        let Some(source) =
            sunrise_gameplay_common::game::api_values::placement_source_into_engine(context.source)
        else {
            return Ok(None);
        };
        if !context.hit.iter().all(|n| n.is_finite())
            || !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
            || !world.is_loaded_at(x, y, z)
        {
            return Ok(None);
        }
        let Some(block) = sunrise_gameplay_common::game::api_values::behavior_block(
            block,
            self.common.res.behaviours(),
        ) else {
            return Ok(None);
        };
        let item = match context.item {
            Some(key) => {
                let Some(item) = mod_api::__private::item_key_id(key) else {
                    return Ok(None);
                };
                item
            }
            None => "",
        };
        self.preview_state_for_placement(
            block,
            &PlacementContext {
                pos: context.pos,
                hit_pos: context.hit_pos,
                face: context.face,
                hit: Vec3::new(context.hit[0], context.hit[1], context.hit[2]),
                inside: context.inside,
                source,
                item: item.to_owned(),
                replace_clicked: context.replace_clicked,
            },
        )
        .map_err(sunrise_gameplay_common::game::api_values::placement_refusal_into_api)
    }

    /// A chosen state written as a placement writes it, then `Block.setPlacedBy`, with no
    /// player: the write and the extra halves are those of `player_place_block`. `Ok(false)`
    /// when the write was refused or changed nothing.
    pub fn level_place_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> Result<bool, LevelWriteError> {
        let flags = placement_flags(update_neighbour_shapes);
        if !self.native_level_write_position(level, pos)?
            || self.common.world.block(pos[0], pos[1], pos[2]) == state
            || !self.placement_allowed(pos, state, Some(flags))?
        {
            return Ok(false);
        }
        if !self.level_set_block(level, pos, state, flags)? {
            return Ok(false);
        }
        self.run_mod_placement_callback(pos, state, None);
        Ok(true)
    }
    /// `Block.setPlacedBy` of a block already standing at `pos`, with no placer. Its stack
    /// is consulted, not placed: the caller has already loaded any block entity data.
    pub(in crate::game) fn level_set_placed_by(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: Option<&ItemStack>,
    ) -> Result<bool, LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if !self.placement_allowed(pos, state, None)? {
            return Ok(false);
        }
        self.run_mod_placement_callback(pos, state, stack);
        Ok(true)
    }

    fn placement_allowed(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        write_flags: Option<BlockUpdateFlags>,
    ) -> Result<bool, LevelWriteError> {
        if self.common.world.carrier_fences.has_locked_cells() {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            if self.common.world.carrier_fences.locks_cell(&pos)
                || self
                    .preview_set_placed_by(at, state, write_flags.map(|flags| flags.bits()))
                    .keys()
                    .any(|p| {
                        self.common
                            .world
                            .carrier_fences
                            .locks_cell(&[p.x, p.y, p.z])
                    })
            {
                return Ok(false);
            }
        }
        if !self.common.transactions.is_empty() && self.wither_spawn_pattern(pos, state).is_some() {
            // The pattern's flag-2 writes, destroy events and neighbour pass are not journalled.
            return Err(LevelWriteError::InTransaction);
        }
        Ok(true)
    }

    /// Chosen-state mod placements did not run the simulation's block-item placement.
    /// Apply its state-only callbacks before the same game-level step used by an item.
    fn run_mod_placement_callback(
        &mut self,
        pos: [i32; 3],
        expected: BlockStateId,
        stack: Option<&ItemStack>,
    ) {
        let [x, y, z] = pos;
        let placed = self.common.world.block(x, y, z);
        let behaviours = self.common.res.behaviours();
        if behaviours.block(placed) != behaviours.block(expected) {
            return;
        }
        if self.captures_placement() {
            if let Some(level) = self.level(self.dim.id()) {
                let _ = self.stage_placement_callback(level, pos, placed, stack);
            }
            return;
        }
        self.with_level(|b, level, ticks| {
            b.set_placed_by_state(level, ticks, BlockPos::new(x, y, z), placed);
        });
        let _ = self.set_placed_by(pos, None, stack);
        let _ = self.server_process_level_events();
    }

    /// The game-level part of `Block.setPlacedBy`, once the state-only part has run.
    /// A supplied player is already the acting player. A supplied stack is read only.
    pub(in crate::game) fn set_placed_by(
        &mut self,
        pos: [i32; 3],
        placer: Option<PlayerRef>,
        stack: Option<&ItemStack>,
    ) -> sunrise_game_data::Result<()> {
        let [x, y, z] = pos;
        let key = (x, y, z);
        let placed = self.common.world.block(x, y, z);
        if self.common.res.behaviours().data(placed).is(Class::Jukebox) {
            if let Some(stack) = stack {
                self.jukebox_placed(pos, stack);
            }
        } else {
            match self.common.world.block_entities.get(&key) {
                Some(BlockEntity::CommandBlock(_)) => self.command_block_placed(key, stack),
                // StructureBlock.createdBy accepts any living placer; this route's placers are players.
                Some(BlockEntity::StructureBlock(_)) if placer.is_some() => {
                    self.structure_block_placed(key)
                }
                // The sign editor is a ServerPlayer side effect, not a block-state or NBT write.
                Some(BlockEntity::Sign(sign))
                    if placer.is_some()
                        && !sign.waxed
                        && sign.front.has_editable_text()
                        && !self.fake_blocks(mod_api::FakeEffect::Menu) =>
                {
                    self.server_request_sign_editor(key, true);
                }
                _ => {}
            }
        }
        self.check_wither_spawn(pos)?;
        if self.common.is_logical_server()
            && block_listens(HookId::SetPlacedBy, placed)
            && let Some(level) = self.level(self.common.dim.id())
        {
            self.native_set_placed_by(level, pos, placed, placer.map(PlayerRef::entity), stack);
        }
        Ok(())
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::remote::dry_level::DryLevelGameExt1 as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
