//! `Level.tickBlockEntities` for the block entities ticked together here
//! (hoppers, crafters, jukeboxes, beacons, creaking hearts, spawners, trial
//! spawners, vaults, beehives, furnaces and potent sulfur): only those whose state has a
//! ticker, in vanilla's list order (`sunrise_client::block_entities`), in
//! chunks that tick blocks. Sculk, brewing stands and pistons tick in their
//! own passes.
use crate::game::ServerGame;
use sunrise_core::position::BlockPos;
#[allow(unused_imports)]
use sunrise_game_data::Result;
use sunrise_gameplay::block_entity_ticks::{block_ticking_at, ticker_name};
use sunrise_gameplay_common::game::published_loops::{Attributed, LoopFlow, LoopMod};
use sunrise_level::block_entity_ticks::Ticker;
#[allow(unused_imports)]
use sunrise_level::{
    block_entities::{Pos, Ticking},
    containers::{BlockEntity, HOPPER_SLOTS},
};

/// What `block_entities.tick` is visiting: the kinds its dispatch ticks, read once per entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickedKind {
    /// `HopperBlockEntity` (its transfer state is `Game::hoppers[pos]`).
    Hopper,
    /// `CrafterBlockEntity`.
    Crafter,
    /// `JukeboxBlockEntity`, while it holds a record.
    Jukebox,
    /// `BeaconBlockEntity`.
    Beacon,
    /// `CreakingHeartBlockEntity`.
    CreakingHeart,
    /// `SpawnerBlockEntity`.
    Spawner,
    /// `TrialSpawnerBlockEntity`.
    TrialSpawner,
    /// `VaultBlockEntity`.
    Vault,
    /// `BeehiveBlockEntity` (beehives and bee nests).
    Beehive,
    /// `AbstractFurnaceBlockEntity` (furnaces, blast furnaces, smokers).
    Furnace,
    /// `BrewingStandBlockEntity`.
    BrewingStand,
    /// `DaylightDetectorBlockEntity`.
    DaylightDetector,
    /// `TheEndGatewayBlockEntity`.
    EndGateway,
    /// `CampfireBlockEntity`.
    Campfire,
    /// A mod's ticking block entity.
    Mod,
    /// `PotentSulfurBlockEntity`.
    PotentSulfur,
}

impl TickedKind {
    /// What `block_entities.tick` ticks `entity` as; `None` for a block entity it does not tick.
    #[inline(always)]
    pub(in crate::game) fn of(entity: &BlockEntity) -> Option<Self> {
        Some(match entity {
            BlockEntity::Chest(items) if items.len() == HOPPER_SLOTS => Self::Hopper,
            BlockEntity::Crafter(_) => Self::Crafter,
            BlockEntity::Jukebox(_) => Self::Jukebox,
            BlockEntity::Beacon(_) => Self::Beacon,
            BlockEntity::CreakingHeart(_) => Self::CreakingHeart,
            BlockEntity::Spawner(_) => Self::Spawner,
            BlockEntity::TrialSpawner(_) => Self::TrialSpawner,
            BlockEntity::Vault(_) => Self::Vault,
            BlockEntity::Beehive(_) => Self::Beehive,
            BlockEntity::Furnace(_) => Self::Furnace,
            BlockEntity::Brewing(_) => Self::BrewingStand,
            BlockEntity::DaylightDetector => Self::DaylightDetector,
            BlockEntity::EndGateway(_) => Self::EndGateway,
            BlockEntity::Campfire(_) => Self::Campfire,
            BlockEntity::Mod(_) => Self::Mod,
            BlockEntity::PotentSulfur(_) => Self::PotentSulfur,
            _ => return None,
        })
    }
    /// [`Self::of`], with a mod's block entity's type: its numeric wire ID, kept on the entity
    /// since it was made (`ModBlockEntity::numeric_id`).
    #[inline(always)]
    pub(in crate::game) fn with_type(entity: &BlockEntity) -> Option<(Self, Option<u32>)> {
        let kind = Self::of(entity)?;
        let mod_type = match entity {
            BlockEntity::Mod(entity) => entity.numeric_id(),
            _ => None,
        };
        Some((kind, mod_type))
    }
}

/// The block entity `block_entities.tick` is visiting, as a slot gets it: the loop's own state,
/// lent under the loop's `&mut self` for one call. Pinned to this build like a mixin's target.
#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
pub struct BlockEntityTick<'g, 'a> {
    /// The game the loop runs on: the level being ticked, `hoppers`, everything the loop has.
    pub game: &'g mut crate::game::ServerGame<'a>,
    /// The block entity's position.
    pub pos: Pos,
    /// What it is, from the block entity the dispatch is about to tick.
    pub kind: TickedKind,
    /// A mod's block entity's type, as a numeric wire ID; read through [`OwnBlockEntities`].
    pub mod_type: Option<u32>,
}

#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
impl BlockEntityTick<'_, '_> {
    /// The visited block entity.
    pub(crate) fn block_entity(&mut self) -> Option<&mut BlockEntity> {
        self.game.world.block_entities.get_mut(&self.pos)
    }
}

/// A mod's own block entity types, for a slot of `block_entities.tick` to pick out its mod's block
/// entities (book: Published loops). Made from the types' ids; the first [`Self::contains`]
/// resolves them once into a flag for each numeric wire ID. A mod block entity carries its own
/// type's numeric wire ID from when it was made, whatever block it stands on, so each visit after
/// that reads one flag: no string work, no hashing, no allocation. A mod with no slot never makes
/// one.
///
/// Both the flags and the entities' IDs assume the block entity type table is frozen for the
/// process (`block_entity_types::install` refuses a change; a new table needs a restart), and so are
/// the block states the types were made valid on; nothing here reads a block state.
#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
pub(crate) struct OwnBlockEntities {
    /// The types' ids, as the mod registered them (`mymod:machine`).
    pub(in crate::game) ids: &'static [&'static str],
    /// By numeric wire ID, whether the type is one of `ids`; `None` until a visit finds the game's
    /// type table.
    pub(in crate::game) by_numeric_id: Option<Box<[bool]>>,
}

#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
impl OwnBlockEntities {
    /// The block entity types `ids`, such as `&["mymod:machine"]`; resolved on the first visit,
    /// which logs a warning for each id the game has no block entity type of. Such an id matches
    /// nothing.
    pub(crate) const fn new(ids: &'static [&'static str]) -> Self {
        Self {
            ids,
            by_numeric_id: None,
        }
    }
    /// Whether the visited block entity is a mod's of one of these types. Before the type table is
    /// installed nothing is resolved or kept, and a later visit resolves.
    #[inline]
    pub(crate) fn contains(&mut self, tick: &BlockEntityTick<'_, '_>) -> bool {
        let Some(mod_type) = tick.mod_type else {
            return false;
        };
        if self.by_numeric_id.is_none() {
            self.by_numeric_id = Self::resolve(self.ids);
        }
        self.by_numeric_id
            .as_ref()
            .and_then(|by_numeric_id| by_numeric_id.get(mod_type as usize))
            .copied()
            .unwrap_or(false)
    }
    /// For each numeric wire ID, whether it names one of `ids`; `None` while the game has no type
    /// table, or an empty one.
    #[cold]
    pub(in crate::game) fn resolve(ids: &[&str]) -> Option<Box<[bool]>> {
        let types = sunrise_game_data::block_entity_types::current()?;
        if types.numeric_id_bound() == 0 {
            return None;
        }
        let mut by_numeric_id = vec![false; types.numeric_id_bound()];
        for id in ids {
            match types.numeric_id(id) {
                Some(numeric_id) => by_numeric_id[numeric_id as usize] = true,
                None => log::warn!(
                    "Published loop block_entities.tick: OwnBlockEntities names `{id}`, which is no \
                     registered block entity type; it matches nothing"
                ),
            }
        }
        Some(by_numeric_id.into())
    }
}

/// A `[[native.loop]]` slot of `block_entities.tick` (book: Published loops). The loop visits the
/// block entities in vanilla's order, only those in block-ticking chunks, inside the border, live and
/// not asleep, and asks each slot about each. Pinned to this build like a mixin's target: its
/// signatures and the cursor may change in any build.
///
/// The slots get the whole game, so they can change the block entities. The pass visits the list
/// it took when it started: one a slot adds joins the list for the next pass, as one placed by a
/// tick does, and one it removes (or replaces) later in the list is passed over when the loop gets
/// there, as one removed by an earlier tick is. For the visited one, see [`Self::before`].
pub trait BlockEntityTickSlot {
    /// Whether [`Self::before`] may change the visited block entity, so the loop checks it again
    /// after a [`LoopFlow::Continue`]. False only for `()`, the game's own loop, whose `before`
    /// does nothing, so it has no check; a mod's slot always has it.
    const RECHECK: bool = true;
    /// Just before the visited block entity ticks. [`LoopFlow::Skip`] leaves it out this tick;
    /// [`LoopFlow::Stop`] ends the pass (what follows the loop, such as shulker box lids, still runs).
    /// After [`LoopFlow::Continue`] from every slot the loop looks again: when the visited block
    /// entity was removed, replaced by one of another [`TickedKind`] or a mod's of another type,
    /// or a mod block entity is now on an incompatible block, it does not tick and no slot's
    /// [`Self::after`] is called for it.
    #[inline(always)]
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        let _ = tick;
        LoopFlow::Continue
    }
    /// Just after the visited block entity ticked; not called if its tick was skipped.
    #[inline(always)]
    fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
        let _ = tick;
    }
}

/// No mod: the game's own loop.
impl BlockEntityTickSlot for () {
    const RECHECK: bool = false;
}

/// Two slots in order: `A` first, and `B` only while `A` answers [`LoopFlow::Continue`].
impl<A: BlockEntityTickSlot, B: BlockEntityTickSlot> BlockEntityTickSlot for (A, B) {
    const RECHECK: bool = A::RECHECK || B::RECHECK;
    #[inline(always)]
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        match self.0.before(tick) {
            LoopFlow::Continue => self.1.before(tick),
            answer => answer,
        }
    }
    #[inline(always)]
    fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
        self.0.after(tick);
        self.1.after(tick);
    }
}

impl<N: LoopMod, T: BlockEntityTickSlot> BlockEntityTickSlot for Attributed<N, T> {
    #[inline(always)]
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        self.call(|slot| slot.before(tick))
            .unwrap_or(LoopFlow::Continue)
    }
    #[inline(always)]
    fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
        self.call(|slot| slot.after(tick));
    }
}

/// A pass's working state: the item and mob caches and the spawners ticked so far.
pub(in crate::game) type PassScratch = (
    Option<sunrise_gameplay_common::hoppers::ItemColumns>,
    Option<crate::game::potent_sulfur::PassMobs>,
    Vec<Pos>,
    Vec<Pos>,
);
/// Puts the enclosing pass's scratch back when the pass inside it ends, however it ends:
/// a slot may catch a nested pass's panic and let the enclosing pass go on.
pub(in crate::game) struct Enclosing<'g, 'a> {
    pub(in crate::game) game: &'g mut crate::game::ServerGame<'a>,
    pub(in crate::game) scratch: Option<PassScratch>,
}

impl ServerGame<'_> {
    /// The published loop `block_entities.tick` (`published_loops.rs`): with no
    /// `[[native.loop]]` mod its slot is `()`, and a mod build installs its own copy of this
    /// function, with the mods' slots, through its replacement cell. A sleeping hopper is
    /// still visited for its transfer state (cooldown and `tickedGameTime`) but is not handed
    /// to the slot: mod loop listeners see it again once something wakes it.
    #[sunrise_mod_api_macros::sunrise_loop(name = "block_entities.tick", side = "server")]
    pub fn tick_block_entities(&mut self) -> Result<()> {
        self.tick_block_entities_with(&mut ())
    }
    /// `tick_block_entities` with `slot` asked before and after each block entity's tick. Generic
    /// over the slot only, so it has no replacement cell of its own: a mod build compiles it.
    ///
    /// A run from inside a pass (a slot that ticks the block entities again) is a whole pass of
    /// its own, the work after the loop included. It sets the enclosing pass's scratch (the item
    /// and mob caches, the spawners ticked so far) aside and puts it back when it ends, so the
    /// enclosing pass goes on with its own.
    pub fn tick_block_entities_with<S: BlockEntityTickSlot>(&mut self, slot: &mut S) -> Result<()> {
        // Only a pass in progress holds item columns: every pass drops them as it ends.
        let scratch = self.server.item_columns.is_some().then(|| {
            (
                self.server.item_columns.take(),
                self.server.pass_mobs.take(),
                std::mem::take(&mut self.common.spawner_tick_positions),
                std::mem::take(&mut self.common.trial_spawner_tick_positions),
            )
        });
        // The pass below is written on `this`, the game the guard lends back.
        let enclosing = Enclosing {
            game: self,
            scratch,
        };
        let this = &mut *enclosing.game;
        this.fire_pending_block_entity_loads();
        this.tick_craft_table();
        this.tick_container_reach();
        this.ensure_stateful_entity_table();
        if !this.world.block_entities.has_states() {
            this.bind_server_ticking_states();
            let containers = this.native_containers();
            this.world.block_entities.set_mod_containers(containers);
            let fluids = this.native_fluid_tables();
            this.world.block_entities.set_mod_fluids(fluids);
            let energy = this.native_energy_tables();
            this.world.block_entities.set_mod_energy(energy);
        }
        // Only lingering cooldowns are dropped; a hopper that ticks checks
        // its own block. Once a second is soon enough for the rest.
        if this.world.game_time % 20 == 0 {
            this.forget_gone_hoppers();
        }
        // Items and vehicles have settled for this tick, mobs included: a
        // sleeping hopper they now reach (even one dropped after
        // `hoppers_catch_items`) wakes before the hoppers tick.
        this.wake_hoppers_near_items();
        this.wake_hoppers_near_vehicles();
        let ticking = this.block_ticking_area();
        this.spawner_tick_positions.clear();
        this.trial_spawner_tick_positions.clear();
        this.server.item_columns = Some(Default::default());
        this.server.pass_mobs = Some(Default::default());
        let tickers = this.world.block_entity_tickers();
        let mut last_chunk = (i32::MIN, i32::MIN);
        let mut last_chunk_ticking = false;
        for ticker in tickers.iter().copied() {
            let pos = ticker.0;
            let chunk = (pos.0 >> 4, pos.2 >> 4);
            let chunk_ticking = if chunk == last_chunk {
                last_chunk_ticking
            } else {
                last_chunk = chunk;
                last_chunk_ticking = ticking(pos.0, pos.2);
                last_chunk_ticking
            };
            if !chunk_ticking {
                continue;
            }
            // Ticks before this one can remove it (`TickingBlockEntity.isRemoved`).
            let live = this.world.block_entities.is_live(ticker);
            let inside = this.border.contains(f64::from(pos.0), f64::from(pos.2));
            if this.world.block_entities.is_asleep(&pos) {
                // A sleeping hopper's slot still passes: its cooldown and
                // `tickedGameTime` advance as vanilla's do, without a transfer
                // attempt, but only where its ticker would run. The published
                // loop therefore does not visit it.
                if live && inside && this.world.block_entities.is_hopper_asleep(&pos) {
                    this.tick_sleeping_hopper(pos);
                }
                continue;
            }
            if !live || !inside {
                continue;
            }
            let Some(entity) = this.world.block_entities.get(&pos) else {
                continue;
            };
            let Some((kind, mod_type)) = TickedKind::with_type(entity) else {
                continue;
            };
            if matches!(entity, BlockEntity::Mod(m) if mod_api::__private::block_entity_owner_disabled(m.id()))
            {
                continue;
            }
            match slot.before(&mut BlockEntityTick {
                game: this,
                pos,
                kind,
                mod_type,
            }) {
                LoopFlow::Continue => {}
                LoopFlow::Skip => continue,
                LoopFlow::Stop => break,
            }
            // `before` had the game: the entity it was asked about may be gone, another kind or a
            // mod's of another type.
            if S::RECHECK
                && (!this.world.block_entities.is_live(ticker)
                    || this
                        .world
                        .block_entities
                        .get(&pos)
                        .and_then(TickedKind::with_type)
                        != Some((kind, mod_type)))
            {
                continue;
            }
            sunrise_threads::work::set_chunk(chunk.0, chunk.1);
            sunrise_threads::work::set_entity(sunrise_threads::work::entity_id(
                this.world.block_entities.get(&pos).map_or("", ticker_name),
            ));
            match kind {
                TickedKind::Hopper => this.tick_hopper(pos),
                TickedKind::Crafter => this.tick_crafter(pos),
                TickedKind::Jukebox => this.tick_jukebox(pos),
                TickedKind::Beacon => this.tick_beacon_at(pos),
                TickedKind::CreakingHeart => this.tick_creaking_heart(pos)?,
                TickedKind::Spawner => {
                    this.tick_spawner(pos)?;
                    this.spawner_tick_positions.push(pos);
                }
                TickedKind::TrialSpawner => {
                    this.tick_trial_spawner(pos)?;
                    this.trial_spawner_tick_positions.push(pos);
                }
                TickedKind::Vault => this.tick_vault(pos)?,
                TickedKind::Beehive => this.tick_beehive(pos),
                TickedKind::Furnace => this.tick_furnace(pos)?,
                TickedKind::BrewingStand => this.tick_brewing_stand(pos)?,
                TickedKind::DaylightDetector => this.tick_daylight_detector(pos),
                TickedKind::EndGateway => this.tick_end_gateway(pos),
                TickedKind::Campfire => this.tick_campfire(pos)?,
                TickedKind::Mod => {
                    if !this.tick_mod_block_entity(pos) {
                        continue;
                    }
                }
                TickedKind::PotentSulfur => {
                    this.tick_potent_sulfur(pos)?;
                }
            }
            slot.after(&mut BlockEntityTick {
                game: this,
                pos,
                kind,
                mod_type,
            });
        }
        sunrise_threads::work::clear_work();
        // Items move between passes: the columns hold for this one only.
        this.server.item_columns = None;
        this.server.pass_mobs = None;
        this.tick_sign_editors();
        this.scan_for_hearts();
        // `ShulkerBoxBlockEntity.tick`: the lids of the boxes this level
        // animates. Their grown collision shape reaches the collision queries
        // through `World::shulkers`, and an opening lid pushes what it runs
        // into (`moveCollidedEntities`), as a moving piston does.
        let pushes = this.tick_shulkers();
        this.sync_shulkers();
        this.push_shulker_entities(pushes);
        Ok(())
    }
    /// `ShulkerBoxBlockEntity.tick` for the boxes in block-ticking chunks, in
    /// position order: the lid's new progress, and the swept box of an opening
    /// lid for `moveCollidedEntities`. A box whose block is gone is dropped.
    /// The boxes are those the level animates: a box opens (`start_open` and
    /// block event 1) before it has one, exactly as vanilla's block entity is
    /// there from the start.
    pub(in crate::game) fn tick_shulkers(&mut self) -> Vec<super::piston_push::ShulkerPush> {
        let ticking = self.block_ticking_area();
        let mut pushes = Vec::new();
        let mut gone = Vec::new();
        let positions: Vec<[i32; 3]> = self.server.be_anims.shulkers.keys().copied().collect();
        for pos in positions {
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            let Some(Ticker::Shulker { facing }) = self.server.be_anims.ticker(state.0) else {
                gone.push(pos);
                continue;
            };
            if !ticking(pos[0], pos[2])
                || !self
                    .common
                    .border
                    .contains(f64::from(pos[0]), f64::from(pos[2]))
            {
                continue;
            }
            let tick = self
                .server
                .be_anims
                .shulkers
                .get_mut(&pos)
                .map(|shulker| shulker.tick(pos, facing));
            let Some(tick) = tick else { continue };
            // `ShulkerBoxBlockEntity.doNeighborUpdates` at the start and the
            // end of an opening or closing lid: the box's collision shape
            // moved, so the neighbours' shapes and their redstone updates run.
            for _ in 0..tick.neighbor_updates {
                let state = self.common.world.block(pos[0], pos[1], pos[2]);
                let at = BlockPos::new(pos[0], pos[1], pos[2]);
                self.with_level(|b, level, ticks| {
                    b.shulker_neighbor_updates(level, ticks, at, state)
                });
            }
            if let Some(push) = tick.push {
                pushes.push((pos, facing, push));
            }
        }
        for pos in gone {
            self.server.be_anims.shulkers.remove(&pos);
        }
        pushes
    }
    /// Single-position read without copying the pending-entity set.
    pub(in crate::game) fn block_ticking_at(&self, x: i32, z: i32) -> bool {
        block_ticking_at(
            &self.server.tickets.0.levels(),
            &self.common.world.entities_pending,
            x,
            z,
        )
    }
    /// `DaylightDetectorBlock.tickEntity` of the detector at `pos`.
    pub(in crate::game) fn tick_daylight_detector(&mut self, pos: Pos) {
        // It updates every 20 game ticks (the behaviour checks this too).
        if self.common.world.game_time % 20 != 0 {
            return;
        }
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        self.with_level(|b, level, ticks| b.tick_daylight_detector(level, ticks, at));
        let _ = self.server_process_level_events();
    }
}

/// Slots that pick out their own mod's block entities (`OwnBlockEntities`).
#[cfg(any())]
pub mod own_block_entity_tests {
    use super::{BlockEntityTick, BlockEntityTickSlot, OwnBlockEntities, TickedKind};

    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
    use mod_api::*;
    use std::sync::{Arc, OnceLock};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::published_loops::LoopFlow;
    use sunrise_level::block_entities::Pos;
    use sunrise_level::containers::{BlockEntity, ModBlockEntity};

    #[cfg(test)]
    /// A mod whose block `block` makes its ticking block entity type of the same id, which is
    /// also valid on the block `also_on` (`BlockEntityType.validBlocks`, added by this mod).
    struct Machines {
        block: &'static str,
        also_on: Option<&'static str>,
        keys: OnceLock<(BlockEntityTypeKey, BlockKey)>,
    }

    #[cfg(test)]
    /// Counts what it is asked about, and notes those of its own types.
    struct Count {
        own: OwnBlockEntities,
        visited: u32,
        mine: Vec<Pos>,
    }

    /// The block `block` at `pos` with a new block entity of the mod type `kind` on it, as a save
    /// that holds it loads it.
    pub fn place_mod_entity(
        game: &mut crate::game::ServerGame<'_>,
        pos: Pos,
        block: &str,
        kind: &str,
    ) {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        let entity = ModBlockEntity::new(kind.into(), Default::default());
        game.world
            .block_entities
            .insert(pos, BlockEntity::Mod(Box::new(entity)));
    }

    /// The data of the mod block entity at `pos`: a [`mark`] for each of its ticks.
    pub fn ticks_of(game: &crate::game::ServerGame<'_>, pos: Pos) -> Vec<i32> {
        match game.world.block_entities.get(&pos) {
            Some(BlockEntity::Mod(entity)) => entity.data.clone(),
            _ => panic!("no mod block entity at {pos:?}"),
        }
    }

    #[cfg(test)]
    /// Once, when asked about the block entity at `at`, replaces it in place (the loop's ticker
    /// stays live) with a new one of mod B's type and answers Continue. Notes the entities it
    /// sees after their tick.
    struct Swap {
        at: Option<Pos>,
        after: Vec<Pos>,
    }

    #[cfg(test)]
    /// Notes each entry, as [`Entry`], before and after its tick.
    struct Seen {
        a: OwnBlockEntities,
        b: OwnBlockEntities,
        before: Vec<Entry>,
        after: Vec<Entry>,
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1 as _;
