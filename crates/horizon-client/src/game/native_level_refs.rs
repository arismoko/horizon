//! A level's blocks and time for native server mods, through a `LevelRef`
//! (docs/native-mods.md, "Entity and level handles", #7387). They read any
//! level the server has loaded, the one in place or another, and return
//! `None` (or `false`) on a client, for a handle of an earlier open, or for a
//! block in a chunk that isn't loaded.
use super::Game;
use horizon_client::mod_api::{
    BlockStateId, ChunkRef, ChunkStatus, LevelRef, NativeTicketOwner, TicketController,
};
use horizon_client::mod_chunk_tickets::ModTicket;
use horizon_client::{streaming::Streaming, world::World};
use horizon_core::position::ChunkPos;

/// Whether chunk `pos` lies inside the world (`/forceload`'s 30 million block bound).
fn chunk_in_world(pos: [i32; 2]) -> bool {
    const LIMIT: i32 = 30_000_000 >> 4;
    pos.iter().all(|c| (-LIMIT..LIMIT).contains(c))
}

/// Whether the mod running owns `controller`: a mod uses only the controllers it registered.
fn calling_mod_owns(controller: TicketController) -> bool {
    controller
        .owner()
        .is_some_and(|owner| horizon_client::crash::current_native_mod() == Some(owner))
}

impl Game<'_> {
    /// The world a handle names now, or None for a stale or foreign one.
    pub(super) fn native_level_world(&self, level: LevelRef) -> Option<&World> {
        if !self.is_logical_server() || self.native_lifecycle.epoch() != Some(level.epoch()) {
            return None;
        }
        if level.dimension() == self.dim.id() {
            return Some(&self.world);
        }
        self.levels.parked_world_of(level.dimension())
    }
    /// Find a loaded chunk in an open server level.
    pub(super) fn native_chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef> {
        self.native_level_world(level)?
            .chunks
            .contains_key(&(pos[0], pos[1]))
            .then_some(ChunkRef { level, pos })
    }
    /// Whether this chunk is loaded; false on a client or for a stale handle.
    pub(super) fn native_chunk_ref_loaded(&self, chunk: ChunkRef) -> bool {
        self.native_chunk(chunk.level(), chunk.pos()).is_some()
    }
    /// The status of a loaded chunk. Proto chunks are not exposed here.
    pub(super) fn native_chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus> {
        self.native_chunk(chunk.level(), chunk.pos())
            .map(|_| ChunkStatus::Full)
    }
    /// A handle for chunk `pos` of an open level, loaded or not, so a mod can
    /// force an unloaded chunk. None for a stale handle, on a client, and for
    /// a chunk outside the world (`ChunkPos` past 30 million blocks).
    pub(super) fn native_level_chunk_ref(
        &self,
        level: LevelRef,
        pos: [i32; 2],
    ) -> Option<ChunkRef> {
        self.native_level_world(level)?;
        chunk_in_world(pos).then_some(ChunkRef::new(level, pos))
    }
    /// The streaming of the level a handle names now, or None for a stale or
    /// foreign one.
    fn native_level_streaming(&self, level: LevelRef) -> Option<&Streaming> {
        self.native_level_world(level)?;
        if level.dimension() == self.dim.id() {
            return Some(&self.streaming);
        }
        self.levels.parked_streaming_of(level.dimension())
    }
    /// Whether `/forceload` holds the chunk (`ServerLevel.getForcedChunks`),
    /// loaded or not; a mod's own ticket on it does not count. None for a stale handle or on a client.
    pub(super) fn native_chunk_forced(&self, chunk: ChunkRef) -> Option<bool> {
        let [x, z] = chunk.pos();
        Some(
            self.native_level_streaming(chunk.level())?
                .commanded()
                .contains(&(x, z)),
        )
    }
    /// `/forceload add` or `remove` of one chunk (`ServerLevel.setChunkForced`):
    /// true when the forced set changed. A forced chunk is kept loaded and
    /// entity ticking, saved with the level, and the level keeps ticking
    /// without a player. This is the world's own `/forceload` state, not a
    /// ticket the mod owns: the chunk stays forced across restarts, and after
    /// the mod is unloaded or removed, until `set_forced(false)` or
    /// `/forceload remove` clears it. Unlike vanilla, the chunk is not loaded before
    /// this returns: it loads over the next ticks, as after `/forceload`.
    /// False for a stale handle, on a client, and for a level the server
    /// has not opened.
    pub(super) fn native_chunk_set_forced(&mut self, chunk: ChunkRef, forced: bool) -> bool {
        if self.native_level_streaming(chunk.level()).is_none() {
            return false;
        }
        let [x, z] = chunk.pos();
        let streaming = if chunk.level().dimension() == self.dim.id() {
            &mut self.streaming
        } else if let Some(streaming) = self
            .levels
            .parked_streaming_of_mut(chunk.level().dimension())
        {
            streaming
        } else {
            return false;
        };
        streaming.set_forced(x, z, forced)
    }
    /// The mods validate the chunk tickets `dim`'s level loaded from its save,
    /// controller by controller (`LoadingValidationCallback.validateTickets`),
    /// and the survivors take hold. Tickets a mod added since the load (in
    /// `level_load`, say) are not candidates, and a controller registered
    /// without a validator keeps all of its tickets. Nothing is built while no
    /// mod listens.
    pub(super) fn validate_mod_chunk_tickets(&mut self, dim: super::Dim) {
        let mut loaded = self.streaming.take_loaded_tickets();
        if !loaded.is_empty()
            && horizon_client::mod_api::listened(horizon_client::mod_api::HookId::ValidateTickets)
        {
            let mut controllers: Vec<u32> = loaded.iter().map(|t| t.controller).collect();
            controllers.sort_unstable();
            controllers.dedup();
            for slot in controllers {
                let Some(controller) = TicketController::from_slot(slot) else {
                    continue;
                };
                let Some(owner) = controller.owner().filter(|_| controller.validates()) else {
                    continue;
                };
                let (own, rest): (Vec<_>, Vec<_>) =
                    loaded.into_iter().partition(|t| t.controller == slot);
                let (mut validation, removals) =
                    horizon_client::mod_chunk_tickets::TicketValidation::new(owner, own.clone());
                self.native_validate_tickets(dim.id(), controller, &mut validation);
                let removed = removals.into_removed();
                loaded = rest;
                loaded.extend(own.into_iter().filter(|t| !removed.contains(t)));
            }
        }
        for ticket in loaded {
            self.streaming.set_mod_forced(ticket, true);
        }
    }
    /// The chunk ticket of `controller` for `owner` on `chunk`
    /// (`TicketController.forceChunk`): true when the controller's tickets
    /// changed. The controller must be the calling mod's own.
    /// A chunk stays forced, entity ticking and saved, and its level keeps
    /// ticking, while any ticket or `/forceload` holds it. With
    /// `natural_spawning` the chunk joins the spawning chunks (inhabited
    /// time, thunder, a spawn attempt) while it lies in the spawning area of a
    /// player (`ChunkMap.collectSpawningChunks`); the local mob cap still lets
    /// mobs spawn only with a non-spectator within 128 blocks. One
    /// owner holds a plain and a natural-spawning ticket on a chunk apart. Tickets are
    /// scoped by (controller, owner, chunk), so two owners in one chunk, or
    /// two controllers, release separately. They are saved in
    /// `data/horizon/mod_chunk_tickets.dat` under the controller's id; when
    /// the world loads, those of a controller no loaded mod registers are
    /// discarded. False outside a mod's hook, for another mod's controller, on
    /// a client, for a chunk outside the world and for a level the server has
    /// not opened.
    pub(super) fn native_chunk_set_mod_forced(
        &mut self,
        chunk: ChunkRef,
        controller: TicketController,
        owner: NativeTicketOwner,
        forced: bool,
        natural_spawning: bool,
    ) -> bool {
        if !calling_mod_owns(controller) {
            return false;
        }
        if self.native_level_streaming(chunk.level()).is_none() || !chunk_in_world(chunk.pos()) {
            return false;
        }
        let [x, z] = chunk.pos();
        let ticket = ModTicket {
            controller: controller.slot(),
            owner,
            chunk: (x, z),
            natural_spawning,
        };
        let dimension = chunk.level().dimension();
        let streaming = if dimension == self.dim.id() {
            &mut self.streaming
        } else if let Some(streaming) = self.levels.parked_streaming_of_mut(dimension) {
            streaming
        } else {
            return false;
        };
        streaming.set_mod_forced(ticket, forced)
    }
    /// The chunk tickets `controller` holds in the level, as (owner, chunk
    /// position, natural_spawning), in order. None for a stale handle, on a
    /// client, outside a mod's hook and for another mod's controller.
    pub(super) fn native_level_mod_forced_chunks(
        &self,
        level: LevelRef,
        controller: TicketController,
    ) -> Option<Vec<(NativeTicketOwner, [i32; 2], bool)>> {
        if !calling_mod_owns(controller) {
            return None;
        }
        let tickets = self.native_level_streaming(level)?.mod_tickets();
        Some(
            tickets
                .of_controller(controller.slot())
                .map(|t| (t.owner, [t.chunk.0, t.chunk.1], t.natural_spawning))
                .collect(),
        )
    }
    /// The level's force-loaded chunks (`/forceload query`), sorted as
    /// the command lists them. None for a stale handle or on a client.
    pub(super) fn native_level_forced_chunks(&self, level: LevelRef) -> Option<Vec<[i32; 2]>> {
        let mut forced: Vec<(i32, i32)> = self
            .native_level_streaming(level)?
            .commanded()
            .iter()
            .copied()
            .collect();
        forced.sort_unstable_by_key(|&(x, z)| ChunkPos::new(x, z).pack());
        Some(forced.into_iter().map(|(x, z)| [x, z]).collect())
    }
    /// The block state ID at `pos` (`Level.getBlockState`), without formatting a string.
    /// None for a chunk that isn't loaded or a `y` outside the world.
    pub(super) fn native_level_state(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<BlockStateId> {
        let world = self.native_level_world(level)?;
        let [x, y, z] = pos;
        let inside = world.shape.contains_y(y);
        (inside && world.chunks.contains_key(&(x >> 4, z >> 4))).then(|| world.block(x, y, z))
    }
    /// The block at `pos` without its properties: `minecraft:oak_stairs`.
    /// None as for `block_state`.
    pub(super) fn native_level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<String> {
        self.native_level_state(level, pos)
            .map(|state| self.res.facts().get(state).name.to_string())
    }
    /// Property `key` of the block state at `pos` (`BlockState.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`.
    pub(super) fn native_level_block_property(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        key: &str,
    ) -> Option<String> {
        let state = self.native_level_state(level, pos)?;
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
    }
    /// Whether this is the level in place: the one the server ticks now, whose block entities
    /// the game's own capabilities read and write and whose capabilities
    /// `LevelRef::capability` looks up. False for a level parked while the server ticks
    /// another (its blocks can still be read), for a handle of an earlier open, and on a
    /// client.
    pub(super) fn native_level_in_place(&self, level: LevelRef) -> bool {
        self.is_logical_server()
            && self.native_lifecycle.epoch() == Some(level.epoch())
            && self.dim.id() == level.dimension()
    }
    /// Whether `pos` is inside the world's height and its chunk is loaded
    /// (`Level.isLoaded`, which is false outside `isInValidBounds`): a block
    /// read or write there finds the world. False on a client or for a stale
    /// handle: the position is not loaded here.
    pub(super) fn native_level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.native_level_world(level).is_some_and(|w| {
            w.shape.contains_y(pos[1]) && w.chunks.contains_key(&(pos[0] >> 4, pos[2] >> 4))
        })
    }
    /// The lowest block `y` and the height of the level
    /// (`LevelHeightAccessor.getMinY`, `getHeight`).
    pub(super) fn native_level_height(&self, level: LevelRef) -> Option<(i32, i32)> {
        self.native_level_world(level)
            .map(|w| (w.shape.min_y, w.shape.height))
    }
    /// The ticks the server has run (`Level.getGameTime`), shared by every
    /// level as in vanilla.
    pub(super) fn native_level_game_time(&self, level: LevelRef) -> Option<i64> {
        self.native_level_world(level)?;
        Some(self.world.game_time)
    }
    /// The time of day in ticks (`Level.getOverworldClockTime`), shared by
    /// every level; it keeps counting past 24000.
    pub(super) fn native_level_day_time(&self, level: LevelRef) -> Option<i64> {
        self.native_level_world(level)?;
        Some(self.world.day_time)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Dim, native_entity_refs::tests::with_game};
    use horizon_client::mod_api::{ChunkRef, NativeWorldEpoch};

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_state_reads_stored_ids_and_refuses_unloaded_positions() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.native_level(game.dim.id()).unwrap();
            let bottom = game.world.bottom_y();
            let top = game.world.top_y();
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let stairs = registry.default_state("minecraft:oak_stairs").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            for (pos, expected) in [([0, bottom, 0], stone), ([15, top - 1, 15], stairs)] {
                game.world.set(pos[0], pos[1], pos[2], expected).unwrap();
                assert_eq!(level.block_state(game, pos), Some(expected));
            }
            assert_eq!(level.block_state(game, [1, bottom, 1]), Some(air));
            assert_eq!(level.block_state(game, [16, bottom, 0]), None);
            assert_eq!(level.block_state(game, [0, bottom - 1, 0]), None);
            assert_eq!(level.block_state(game, [0, top, 0]), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forced_chunks_set_clear_and_query_like_forceload() {
        with_game(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let far = game.native_level_chunk_ref(level, [40, -7]).unwrap();
            let near = game.native_level_chunk_ref(level, [-3, 2]).unwrap();
            assert_eq!(game.native_chunk_forced(far), Some(false));
            assert_eq!(game.native_level_forced_chunks(level), Some(vec![]));
            assert!(game.native_chunk_set_forced(far, true));
            assert!(!game.native_chunk_set_forced(far, true), "already forced");
            assert!(game.native_chunk_set_forced(near, true));
            assert_eq!(game.native_chunk_forced(far), Some(true));
            // The command's own set and order.
            assert!(game.streaming.forced.contains(&(40, -7)));
            let listed = game.native_level_forced_chunks(level).unwrap();
            // Signed packed positions: z first, then x.
            assert_eq!(listed, vec![[40, -7], [-3, 2]]);
            assert!(game.native_chunk_set_forced(far, false));
            assert!(!game.native_chunk_set_forced(far, false), "already clear");
            assert_eq!(game.native_level_forced_chunks(level), Some(vec![[-3, 2]]));
            assert!(!game.streaming.forced.contains(&(40, -7)));
            // Outside the world.
            assert!(game.native_level_chunk_ref(level, [1_875_000, 0]).is_none());
            assert!(
                game.native_level_chunk_ref(level, [0, -1_875_001])
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forced_chunk_changes_the_ticket_sources_and_the_saved_set() {
        with_game(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let chunk = game.native_level_chunk_ref(level, [5, 6]).unwrap();
            assert!(game.native_chunk_set_forced(chunk, true));
            assert!(game.levels_to_tick().contains(&game.dim));
            game.save
                .save_forced_chunks(&game.streaming.forced)
                .unwrap();
            assert!(game.save.load_forced_chunks().contains(&(5, 6)));
            assert!(game.native_chunk_set_forced(chunk, false));
            game.save
                .save_forced_chunks(&game.streaming.forced)
                .unwrap();
            assert!(game.save.load_forced_chunks().is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forced_chunks_of_a_parked_level_and_stale_handles() {
        with_game(|game| {
            let stale_epoch =
                NativeWorldEpoch::new(game.native_lifecycle.epoch().unwrap().get() + 1);
            let level = game.native_level(game.dim.id()).unwrap();
            let stale_level =
                horizon_client::mod_api::LevelRef::new(stale_epoch, level.dimension());
            let stale = ChunkRef::new(stale_level, [0, 0]);
            assert_eq!(game.native_chunk_forced(stale), None);
            assert!(!game.native_chunk_set_forced(stale, true));
            assert_eq!(game.native_level_forced_chunks(stale_level), None);
            assert!(game.native_level_chunk_ref(stale_level, [0, 0]).is_none());
            // A level the server has not opened.
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let parked = horizon_client::mod_api::LevelRef::new(level.epoch(), other.id());
            let chunk = ChunkRef::new(parked, [1, 1]);
            assert_eq!(game.native_chunk_forced(chunk), None);
            assert!(!game.native_chunk_set_forced(chunk, true));
            // Opened, then parked.
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            assert_eq!(game.native_chunk_forced(chunk), Some(false));
            assert!(game.native_chunk_set_forced(chunk, true));
            assert_eq!(game.native_level_forced_chunks(parked), Some(vec![[1, 1]]));
            assert_eq!(game.native_level_forced_chunks(level), Some(vec![]));
            assert!(game.levels_to_tick().contains(&other));
            assert!(game.native_chunk_set_forced(chunk, false));
            assert!(!game.levels_to_tick().contains(&other));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_ref_draws_from_its_levels_own_stream() {
        use horizon_client::mod_api::{LevelRef, ServerHost};
        use horizon_core::random::RandomSource;

        with_game(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            // The level in place: the draws advance `World.random` itself.
            let mut expected = game.world.random.clone();
            let host: &mut dyn ServerHost = game;
            let drawn: Vec<i32> = (0..3)
                .map(|_| level.random(&mut *host).unwrap().next_int())
                .collect();
            let want: Vec<i32> = (0..3).map(|_| expected.next_int()).collect();
            assert_eq!(drawn, want);
            assert_eq!(game.world.random.next_int(), expected.next_int());

            // A parked level answers with its own stream, not the one in place.
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let parked = LevelRef::new(level.epoch(), other.id());
            let host: &mut dyn ServerHost = game;
            assert!(parked.random(&mut *host).is_none(), "not open yet");
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            let mut expected = game
                .levels
                .parked_world_of(other.id())
                .unwrap()
                .random
                .clone();
            let in_place = game.world.random.clone();
            let host: &mut dyn ServerHost = game;
            let drawn = parked.random(&mut *host).unwrap().next_long();
            assert_eq!(drawn, expected.next_long());
            let mut after = game
                .levels
                .parked_world_of(other.id())
                .unwrap()
                .random
                .clone();
            assert_eq!(after.next_long(), expected.next_long());
            let mut untouched = in_place;
            assert_eq!(game.world.random.next_long(), untouched.next_long());

            // A handle of an earlier open names nothing.
            let stale = LevelRef::new(
                NativeWorldEpoch::new(level.epoch().get() + 1),
                game.dim.id(),
            );
            let host: &mut dyn ServerHost = game;
            assert!(stale.random(&mut *host).is_none());
        });
    }

    fn in_mod(id: &'static str) -> horizon_client::crash::NativeModGuard {
        horizon_client::crash::enter_native_mod(horizon_client::crash::NativeModCall {
            id,
            hook: "test",
            part: None,
        })
    }

    use horizon_client::mod_api::{
        self, Hooks, ModEntry, Registrar, ServerHost, TicketController, ValidateTicketsEvent,
        testing,
    };
    use horizon_client::mod_chunk_tickets::{ModTicket, TicketOwner};

    /// Registers three controllers: `quarry` and `drill` validate, `plain` doesn't. `quarry`'s
    /// validation prunes the block at [1, 64, 1]; each validation tells what it was shown.
    struct Machines;
    #[mod_api::__subscriptions]
    impl Hooks for Machines {
        fn register(&self, r: &mut Registrar) {
            r.register_ticket_controller("quarry", true);
            r.register_ticket_controller("machines:drill", true);
            r.register_ticket_controller("plain", false);
        }
        fn validate_tickets(&self, _: &mut dyn ServerHost, e: &mut ValidateTicketsEvent<'_>) {
            SEEN.with_borrow_mut(|seen| {
                seen.push((
                    e.controller.id(),
                    e.validation
                        .block_tickets()
                        .into_iter()
                        .map(|(pos, _)| pos)
                        .collect(),
                ))
            });
            if e.controller.id() == "machines:quarry" {
                e.validation.remove_block_tickets([1, 64, 1]);
            }
        }
    }
    /// Registers one validating controller, and prunes everything it is shown.
    struct Other;
    #[mod_api::__subscriptions]
    impl Hooks for Other {
        fn register(&self, r: &mut Registrar) {
            r.register_ticket_controller("vault", true);
        }
        fn validate_tickets(&self, _: &mut dyn ServerHost, e: &mut ValidateTicketsEvent<'_>) {
            SEEN.with_borrow_mut(|seen| {
                seen.push((
                    e.controller.id(),
                    e.validation
                        .block_tickets()
                        .into_iter()
                        .map(|(pos, _)| pos)
                        .collect(),
                ))
            });
            e.validation.remove_block_tickets([1, 64, 1]);
        }
    }
    thread_local! {
        /// What each validation was shown: its controller, then the block positions.
        static SEEN: std::cell::RefCell<Vec<(&'static str, Vec<[i32; 3]>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn mods() -> horizon_client::mod_api::Mods {
        vec![
            ModEntry::new("machines", Box::new(Machines)),
            ModEntry::new("other", Box::new(Other)),
        ]
        .into()
    }
    fn controller(id: &str) -> TicketController {
        testing::ticket_controller(id).unwrap()
    }
    /// The slot of a registered controller id.
    fn known_slot(id: &str) -> Option<u32> {
        TicketController::from_id(id).map(TicketController::slot)
    }
    fn quarry() -> TicketController {
        controller("machines:quarry")
    }
    /// A game whose mods are `Machines` and `Other`, with their controllers registered.
    fn with_machines(test: impl FnOnce(&mut super::super::Game<'_>)) {
        testing::with_mods(mods(), || with_game(test));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tickets_are_scoped_by_owner_and_share_the_forceload_store() {
        use horizon_client::mod_api::NativeTicketOwner::{Block, Entity};
        with_machines(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let chunk = game.native_level_chunk_ref(level, [5, 6]).unwrap();
            let (a, b) = (Block([80, 64, 96]), Entity(7));
            // Outside a mod's hook there is no caller to own a ticket.
            assert!(!game.native_chunk_set_mod_forced(chunk, quarry(), a, true, false));
            let guard = in_mod("machines");
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), a, true, false));
            assert!(
                !game.native_chunk_set_mod_forced(chunk, quarry(), a, true, false),
                "held"
            );
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), b, true, false));
            assert!(game.streaming.forced.contains(&(5, 6)));
            assert_eq!(
                game.native_chunk_forced(chunk),
                Some(false),
                "not /forceload's"
            );
            assert_eq!(game.native_level_forced_chunks(level), Some(vec![]));
            // Two owners in one chunk release separately.
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), a, false, false));
            assert!(game.streaming.forced.contains(&(5, 6)));
            assert_eq!(
                game.native_level_mod_forced_chunks(level, quarry()),
                Some(vec![(b, [5, 6], false)])
            );
            // Another mod cannot use our controller, however it asks; its own is empty.
            drop(guard);
            let guard = in_mod("other");
            assert_eq!(game.native_level_mod_forced_chunks(level, quarry()), None);
            assert!(!game.native_chunk_set_mod_forced(chunk, quarry(), b, false, false));
            assert!(!game.native_chunk_set_mod_forced(chunk, quarry(), b, true, false));
            let vault = controller("other:vault");
            assert_eq!(
                game.native_level_mod_forced_chunks(level, vault),
                Some(vec![])
            );
            assert!(game.native_chunk_set_mod_forced(chunk, vault, b, true, false));
            assert!(game.native_chunk_set_mod_forced(chunk, vault, b, false, false));
            drop(guard);
            // Our other controller is apart from the first: it neither lists nor releases its tickets.
            let guard = in_mod("machines");
            let drill = controller("machines:drill");
            assert_eq!(
                game.native_level_mod_forced_chunks(level, drill),
                Some(vec![])
            );
            assert!(!game.native_chunk_set_mod_forced(chunk, drill, b, false, false));
            assert!(game.native_chunk_set_mod_forced(chunk, drill, b, true, false));
            assert!(game.native_chunk_set_mod_forced(chunk, drill, b, false, false));
            assert_eq!(
                game.native_level_mod_forced_chunks(level, quarry()),
                Some(vec![(b, [5, 6], false)])
            );
            drop(guard);
            // /forceload and a ticket on one chunk: the chunk stays until both go.
            assert!(game.native_chunk_set_forced(chunk, true));
            assert!(game.native_chunk_set_forced(chunk, false));
            assert!(
                game.streaming.forced.contains(&(5, 6)),
                "the ticket holds it"
            );
            assert!(game.native_chunk_set_forced(chunk, true));
            let _guard = in_mod("machines");
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), b, false, false));
            assert!(
                game.streaming.forced.contains(&(5, 6)),
                "/forceload holds it"
            );
            game.streaming.clear_forced();
            assert!(game.streaming.forced.is_empty());
            // The world's bounds, as /forceload checks them.
            let outside = horizon_client::mod_api::ChunkRef::new(level, [1_875_000, 0]);
            assert!(!game.native_chunk_set_mod_forced(outside, quarry(), a, true, false));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tickets_save_apart_from_vanillas_tickets_and_release_with_the_world() {
        use horizon_client::mod_api::NativeTicketOwner::{Block, Entity};
        with_machines(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let chunk = game.native_level_chunk_ref(level, [-9, 4]).unwrap();
            let _guard = in_mod("machines");
            let drill = controller("machines:drill");
            let (block, entity) = (Block([-140, 70, 70]), Entity(u128::MAX - 3));
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), block, true, false));
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), entity, true, false));
            assert!(game.native_chunk_set_mod_forced(chunk, drill, block, true, false));
            game.save_chunk_tickets().unwrap();
            let vanilla = game.save.load_forced_chunks();
            assert!(vanilla.is_empty(), "vanilla's file has no mod ticket");
            // The world closes: its streaming goes, the next opening reads the file.
            let reopen = |game: &mut super::super::Game<'_>, slot_of: fn(&str) -> Option<u32>| {
                game.streaming.load_forced(
                    game.save.load_forced_chunks(),
                    game.save.load_mod_tickets(slot_of),
                );
                game.validate_mod_chunk_tickets(game.dim);
            };
            reopen(game, known_slot);
            // Each ticket comes back under the controller that held it.
            let listed = |game: &super::super::Game<'_>, c| {
                game.native_level_mod_forced_chunks(level, c).unwrap()
            };
            assert_eq!(listed(game, quarry()).len(), 2);
            assert_eq!(listed(game, drill), vec![(block, [-9, 4], false)]);
            assert!(game.streaming.forced.contains(&(-9, 4)));
            // No loaded mod registers the controller: its tickets are discarded, nothing
            // is forced, and the next save drops them from the file.
            reopen(game, |_| None);
            assert!(game.streaming.forced.is_empty());
            game.save_chunk_tickets().unwrap();
            reopen(game, known_slot);
            assert!(game.streaming.forced.is_empty());
            assert!(game.save.load_mod_tickets(known_slot).is_empty());
            // Released tickets leave the file.
            assert!(game.native_chunk_set_mod_forced(
                chunk,
                quarry(),
                Block([-140, 70, 70]),
                true,
                false
            ));
            assert!(game.native_chunk_set_mod_forced(
                chunk,
                quarry(),
                Block([-140, 70, 70]),
                false,
                false
            ));
            game.save_chunk_tickets().unwrap();
            assert!(game.save.load_mod_tickets(known_slot).is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_ticket_keeps_a_parked_level_active() {
        with_machines(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            let parked = horizon_client::mod_api::LevelRef::new(level.epoch(), other.id());
            let chunk = ChunkRef::new(parked, [2, 2]);
            let _guard = in_mod("machines");
            let owner = horizon_client::mod_api::NativeTicketOwner::Block([32, 64, 32]);
            assert!(!game.levels_to_tick().contains(&other));
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), owner, true, false));
            assert!(game.levels_to_tick().contains(&other));
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), owner, false, false));
            assert!(!game.levels_to_tick().contains(&other));
        });
    }

    /// The result `/forceload query` gives: the chunks listed, or 0 when it reports none.
    fn query_result(game: &mut super::super::Game<'_>, command: &str) -> Option<i32> {
        let d = horizon_client::command::vanilla::game_dispatcher_with_features(
            &game.level.enabled_features,
        );
        let parse = d.parse(command, &game.ui.registries);
        let (ctx, _) = parse.executable().ok()?;
        let (ctx, root) = (ctx.clone(), parse.context.clone());
        game.run_chain(&root, &ctx).ok()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forceload_query_lists_only_command_tickets() {
        with_machines(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let chunk = game.native_level_chunk_ref(level, [3, 3]).unwrap();
            game.ui.registries.permission = 4;
            let _guard = in_mod("machines");
            let owner = horizon_client::mod_api::NativeTicketOwner::Block([48, 64, 48]);
            assert!(game.native_chunk_set_mod_forced(chunk, quarry(), owner, true, false));
            assert!(game.streaming.forced.contains(&(3, 3)));
            assert_eq!(query_result(game, "forceload query"), Some(0), "list form");
            assert_eq!(
                query_result(game, "forceload query 48 48"),
                None,
                "position form"
            );
            assert!(game.native_chunk_set_forced(chunk, true));
            assert_eq!(query_result(game, "forceload query"), Some(1));
            assert_eq!(query_result(game, "forceload query 48 48"), Some(1));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn natural_spawning_tickets_and_plain_ones_save_and_load_apart() {
        use horizon_client::mod_api::NativeTicketOwner::Block;
        with_machines(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let plain = game.native_level_chunk_ref(level, [11, 12]).unwrap();
            let spawning = game.native_level_chunk_ref(level, [13, 12]).unwrap();
            let _guard = in_mod("machines");
            let owner = Block([176, 64, 192]);
            assert!(game.native_chunk_set_mod_forced(plain, quarry(), owner, true, false));
            assert!(game.native_chunk_set_mod_forced(spawning, quarry(), owner, true, true));
            let tickets = game.streaming.mod_tickets();
            assert!(!tickets.forces_natural_spawning((11, 12)));
            assert!(tickets.forces_natural_spawning((13, 12)));
            // Both kinds load and tick: they are forced chunks.
            assert!(game.streaming.forced.contains(&(11, 12)));
            assert!(game.streaming.forced.contains(&(13, 12)));
            assert!(game.levels_to_tick().contains(&game.dim));
            assert_eq!(
                game.native_level_mod_forced_chunks(level, quarry()),
                Some(vec![(owner, [11, 12], false), (owner, [13, 12], true)])
            );
            // One owner holds both kinds on one chunk apart.
            assert!(game.native_chunk_set_mod_forced(plain, quarry(), owner, true, true));
            assert!(game.native_chunk_set_mod_forced(plain, quarry(), owner, false, true));
            assert!(game.streaming.forced.contains(&(11, 12)));
            assert!(
                !game
                    .streaming
                    .mod_tickets()
                    .forces_natural_spawning((11, 12))
            );
            game.save_chunk_tickets().unwrap();
            let saved = game.save.load_mod_tickets(known_slot);
            game.streaming.load_forced(Default::default(), saved);
            game.validate_mod_chunk_tickets(game.dim);
            let tickets = game.streaming.mod_tickets();
            assert!(!tickets.forces_natural_spawning((11, 12)));
            assert!(tickets.forces_natural_spawning((13, 12)));
            assert!(tickets.holds((11, 12)));
        });
    }

    /// The flag adds a chunk to the spawning chunks (inhabited time, a spawn
    /// attempt); the local mob cap still wants a non-spectator within 128
    /// blocks of its centre, as NeoForge (which leaves `LocalMobCapCalculator`
    /// alone) does.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_natural_spawning_ticket_far_from_every_player_runs_the_attempt_but_the_local_cap_refuses()
    {
        use horizon_client::chunk::Chunk;
        use horizon_client::mobs::spawn_selection::{Category, SpawnPlayer, SpawnState};
        use horizon_client::mod_api::NativeTicketOwner::Block;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        use std::sync::Arc;
        with_machines(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            for cx in 6..=10 {
                for cz in 6..=10 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let level = game.native_level(game.dim.id()).unwrap();
            // Both chunks lie in the player's 8-chunk area and past 128 blocks of it.
            let flagged = game.native_level_chunk_ref(level, [8, 8]).unwrap();
            let plain = game.native_level_chunk_ref(level, [8, 7]).unwrap();
            {
                let _guard = in_mod("machines");
                let owner = Block([128, 64, 128]);
                assert!(game.native_chunk_set_mod_forced(flagged, quarry(), owner, true, true));
                assert!(game.native_chunk_set_mod_forced(plain, quarry(), owner, true, false));
            }
            game.spawn_natural();
            assert_eq!(game.world.inhabited.get(&(8, 8)), Some(&1));
            assert!(
                !game.world.inhabited.contains_key(&(8, 7)),
                "without the flag it is no spawning chunk"
            );
            assert!(game.mobs.is_empty(), "the local cap refuses");
            // The cap itself: no player within 128 blocks of the chunk.
            let players: Vec<SpawnPlayer> = game
                .level_players()
                .iter()
                .map(|p| SpawnPlayer {
                    position: [p.position.x, p.position.y, p.position.z],
                    spectator: p.spectator,
                })
                .collect();
            let natural = game.natural_level();
            let mut state = SpawnState::new(&natural, 1, players, &[], |_, _| true);
            assert!(!state.can_spawn_local(Category::Monster, 8, 8));
            assert!(state.can_spawn_local(Category::Monster, 0, 0));
        });
    }

    fn saved(controller: &str, pos: [i32; 3], chunk: (i32, i32)) -> ModTicket {
        ModTicket {
            controller: controller_slot(controller),
            owner: TicketOwner::Block(pos),
            chunk,
            natural_spawning: false,
        }
    }
    fn controller_slot(id: &str) -> u32 {
        controller(id).slot()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn validation_drops_a_removed_ticket_and_leaves_the_rest() {
        with_machines(|game| {
            // `quarry` and `vault` validate; `plain` doesn't. All of them hold the pruned block.
            let tickets = vec![
                saved("machines:quarry", [1, 64, 1], (7, 7)),
                saved("machines:quarry", [9, 64, 9], (8, 8)),
                saved("machines:drill", [1, 64, 1], (9, 9)),
                saved("machines:plain", [1, 64, 1], (10, 10)),
                saved("other:vault", [1, 64, 1], (11, 11)),
            ];
            game.streaming.load_forced(Default::default(), tickets);
            assert!(
                game.streaming.forced.is_empty(),
                "loaded, not yet validated"
            );
            game.validate_mod_chunk_tickets(game.dim);
            let forced = &game.streaming.forced;
            assert!(!forced.contains(&(7, 7)), "quarry's prune");
            assert!(forced.contains(&(8, 8)));
            assert!(
                forced.contains(&(9, 9)),
                "drill's validation prunes nothing"
            );
            assert!(forced.contains(&(10, 10)), "no validator, no validation");
            assert!(!forced.contains(&(11, 11)), "other's own controller");
            // A validation is shown one controller's tickets, whichever mod hears it.
            let seen = SEEN.take();
            let quarry_seen = vec![[1, 64, 1], [9, 64, 9]];
            assert!(seen.contains(&("machines:quarry", quarry_seen.clone())));
            assert!(seen.contains(&("machines:drill", vec![[1, 64, 1]])));
            assert!(seen.contains(&("other:vault", vec![[1, 64, 1]])));
            assert!(
                seen.iter().all(|(id, _)| *id != "machines:plain"),
                "never asked"
            );
            // The mod that doesn't own the controller saw an empty view.
            assert!(
                seen.iter()
                    .any(|(id, tickets)| *id == "machines:quarry" && tickets.is_empty())
            );
            game.save_chunk_tickets().unwrap();
            let file = game.save.load_mod_tickets(known_slot);
            assert_eq!(file.len(), 3);
            assert!(
                !file
                    .iter()
                    .any(|t| t.chunk == (7, 7) || t.chunk == (11, 11))
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn validation_runs_when_a_parked_level_opens_and_not_without_a_listener() {
        testing::with_mods(mods(), || {
            with_game(|game| {
                let other = if game.dim == Dim::Nether {
                    Dim::End
                } else {
                    Dim::Nether
                };
                let world =
                    std::env::temp_dir().join(format!("horizon-validate-{}", std::process::id()));
                game.world_root = world.clone();
                let root = horizon_client::save::dimension_dir(&world, other.id());
                let save = horizon_client::save::SaveDir::new(root, other.shape());
                let mut tickets = horizon_client::mod_chunk_tickets::ModTickets::default();
                tickets.insert(saved("machines:quarry", [1, 64, 1], (7, 7)));
                tickets.insert(saved("machines:quarry", [9, 64, 9], (8, 8)));
                save.save_mod_tickets(&tickets, |slot| {
                    TicketController::from_slot(slot).map(TicketController::id)
                })
                .unwrap();
                let back = game.dim;
                game.enter_level(other).unwrap();
                game.enter_level(back).unwrap();
                let parked = game.levels.parked_streaming_of(other.id()).unwrap();
                assert!(!parked.forced.contains(&(7, 7)));
                assert!(parked.forced.contains(&(8, 8)));
                assert!(game.levels_to_tick().contains(&other));
                let _ = std::fs::remove_dir_all(&world);
            });
        });
        // No mod listens: the tickets stay as loaded.
        struct Idle;
        #[mod_api::__subscriptions]
        impl Hooks for Idle {
            fn register(&self, r: &mut Registrar) {
                r.register_ticket_controller("quarry", true);
            }
        }
        let mods: horizon_client::mod_api::Mods =
            vec![ModEntry::new("idle", Box::new(Idle))].into();
        testing::with_mods(mods, || {
            with_game(|game| {
                let tickets = vec![saved("idle:quarry", [1, 64, 1], (7, 7))];
                game.streaming.load_forced(Default::default(), tickets);
                game.validate_mod_chunk_tickets(game.dim);
                assert!(game.streaming.forced.contains(&(7, 7)));
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn validation_sees_the_saved_tickets_not_ones_added_after_the_load() {
        with_machines(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            let tickets = vec![saved("machines:quarry", [1, 64, 1], (7, 7))];
            game.streaming.load_forced(Default::default(), tickets);
            // `level_load` runs after the load: its hold is the same owner's.
            let fresh = game.native_level_chunk_ref(level, [6, 6]).unwrap();
            {
                let _guard = in_mod("machines");
                let owner = horizon_client::mod_api::NativeTicketOwner::Block([1, 64, 1]);
                assert!(game.native_chunk_set_mod_forced(fresh, quarry(), owner, true, false));
            }
            game.validate_mod_chunk_tickets(game.dim);
            assert!(!game.streaming.forced.contains(&(7, 7)), "saved one pruned");
            assert!(game.streaming.forced.contains(&(6, 6)), "fresh one kept");
        });
    }
}
