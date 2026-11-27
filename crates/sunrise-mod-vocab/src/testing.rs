//! Test support, enabled by the `testing` feature: scoped trait-mod installation
//! ([`with_dispatch`]), and [`FakeHost`], a host built from small fakes so that a mod's tests run
//! without starting the game.
//!
//! ```
//! use mod_api::testing::FakeHost;
//! use mod_api::prelude::ItemsHost;
//! use mod_api::{HostTransaction, ITEMS, ItemResource, ItemStack, ServerHost, resource_util::move_resources};
//!
//! let mut host = FakeHost::new();
//! let level = host.level.level();
//! host.level.block_entity([0, 64, 0]).with_items(1);
//! host.level.block_entity([1, 64, 0]).with_items(1);
//! let stone_id = mod_api::Identifier::parse("minecraft:stone").unwrap();
//! host.registries.resolve_items([stone_id.clone()]).unwrap();
//! let stone = ItemStack::new(host.item_key(&stone_id).unwrap(), 5).unwrap();
//! host.level.block_entity([0, 64, 0]).items.as_mut().unwrap().slots[0] = Some(stone);
//!
//! let server: &mut dyn ServerHost = &mut host;
//! let from = level.capability(server, &ITEMS, [0, 64, 0], None).unwrap();
//! let into = level.capability(server, &ITEMS, [1, 64, 0], None).unwrap();
//! let moved = server.transaction(|host, tx| {
//!     move_resources(host, &from, &into, |_: &ItemResource| true, 3, Some(tx))
//! });
//! assert_eq!(moved, Ok(3));
//! let into = host.level.block_entity_at([1, 64, 0]).unwrap().items.as_ref().unwrap();
//! assert_eq!(into.count("minecraft:stone"), 3);
//! ```
/// Build the command for one exact ignored test in a fresh copy of this test binary.
/// The caller supplies scratch resources and verifies the fixture's completion.
pub fn fresh_case_command(name: &str, flag: &str, case: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            name,
            "--nocapture",
            "--test-threads=4",
        ])
        .env(flag, case);
    command
}

// hidden: only the engine's test platform installs these fixture callbacks.
#[doc(hidden)]
#[path = "testing/engine_platform.rs"]
pub mod engine_platform;
#[path = "testing/journal.rs"]
mod journal;
#[path = "testing/level.rs"]
mod level;
#[path = "testing/players.rs"]
mod players;
#[path = "testing/registries.rs"]
mod registries;
#[path = "testing/scratch.rs"]
mod scratch;
#[path = "testing/services.rs"]
mod services;

/// An item forward reference missing from a fake's load-owned catalog.
pub use crate::parts::ItemKeyError;
pub use journal::FakeJournal;
pub use level::{
    FakeBattery, FakeBlockEntity, FakeContainer, FakeEntity, FakeGameEvent, FakeLevel, FakeSound,
    FakeTank,
};
pub use players::{FakePlayer, FakePlayers, GameProfileFixture};
pub use registries::FakeRegistries;
pub use services::{FakeCommands, FakeMessages};

use crate::item::item_stack::component_edits::edit_item_components;
use crate::item::transaction::{ScratchScopeError, TransactionContext, TransactionScopeError};
use crate::runtime::Installed;
use crate::world::ticket_controller::{ControllerInfo, TEST_CONTROLLERS};
use crate::{
    Backend, BackendMut, BackendMutRef, BackendRef, BlockEntityRef, BlockEntityTypeKey, BlockSide,
    ClientHost, Commands, EntityRef, FluidStack, Host, InventoryView, ItemComponentEditor, ItemKey,
    ItemPlace, ItemPlaceEditError, ItemStack, ItemStackView, LevelRef, Messages, Mods, PlayerRef,
    Players, ScratchSlot, ServerHost, StackWriteError, TEST_MODS, Tag, Text, WorldEpoch,
};

/// The world epoch of every fake's handles.
const FAKE_EPOCH: u64 = 1;

/// Run a closure with these owned mods on this thread, restoring the previous list on exit.
///
/// The test allocation is leaked because dispatch exposes a static slice. Nested scopes and
/// panic unwinding restore the preceding list; other threads are unaffected.
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
pub fn with_dispatch<R>(mods: Mods, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<&'static Installed>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_MODS.set(self.0);
        }
    }
    let installed = Box::leak(Box::new(Installed::new(mods)));
    let _restore = Restore(TEST_MODS.replace(Some(installed)));
    f()
}

/// Lend new registration contexts on this thread a test startup screen, restoring the previous
/// screen on exit or unwind.
// hidden: only test fixtures supply a stand-in startup screen.
#[doc(hidden)]
pub fn with_startup_screen<R>(
    screen: std::sync::Arc<dyn crate::StartupScreen>,
    run: impl FnOnce() -> R,
) -> R {
    use crate::world::startup_progress::TEST_SCREEN;
    struct Restore(Option<std::sync::Arc<dyn crate::StartupScreen>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_SCREEN.replace(self.0.take());
        }
    }
    let _restore = Restore(TEST_SCREEN.replace(Some(screen)));
    run()
}

struct RestoreParts {
    previous: Option<&'static crate::parts::Installed>,
    committed: bool,
}

impl RestoreParts {
    fn fresh() -> Self {
        Self {
            previous: crate::parts::TEST_PARTS.replace(None),
            committed: false,
        }
    }
}

impl Drop for RestoreParts {
    fn drop(&mut self) {
        if !self.committed {
            crate::parts::TEST_PARTS.set(self.previous);
        }
    }
}

/// Probe registrations with no frozen test Parts, then run with the caller's lookup snapshot.
/// Any Parts installed by the body stay local to this scope, even if it unwinds.
// hidden: the API-owned `with_mods` uses this boundary; not API for mod authors.
#[doc(hidden)]
pub fn with_parts_registration_probe<T, R>(
    mods: Mods,
    probe: impl FnOnce(&[crate::ModEntry]) -> T,
    run: impl FnOnce(Mods, T) -> R,
) -> R {
    let restore = RestoreParts::fresh();
    let registered = probe(&mods);
    crate::parts::TEST_PARTS.set(restore.previous);
    run(mods, registered)
}

/// Rebuild a fixture's Parts snapshot from a fresh registration epoch on this thread.
///
/// On success the replacement stays installed. An error or unwinding restores the previous snapshot.
/// Production registries remain frozen; only game fixtures that need a complete key table
/// after their initial synthetic registration should use this.
// hidden: only the shared game fixtures in sunrise-mod-api rebuild their Parts epoch; not API.
#[doc(hidden)]
pub fn rebuild_parts<R, E>(f: impl FnOnce() -> Result<R, E>) -> Result<R, E> {
    let mut restore = RestoreParts::fresh();
    let result = f();
    restore.committed = result.is_ok();
    result
}

#[cfg(test)]
#[test]
fn rebuilding_fixture_parts_commits_success_and_restores_on_error_or_unwind() {
    use crate::parts::{custom_stat_declarations, install_input, installation::InstalledInput};
    let install = |id: &str| {
        install_input(InstalledInput {
            custom_stats: vec![id.into()],
            ..Default::default()
        });
    };
    install("fixture:old");
    let result: Result<(), ()> = rebuild_parts(|| {
        assert!(custom_stat_declarations().is_empty());
        install("fixture:new");
        Ok(())
    });
    assert_eq!(result, Ok(()));
    assert_eq!(custom_stat_declarations(), ["fixture:new"]);
    let result: Result<(), &str> = rebuild_parts(|| {
        install("fixture:failed");
        Err("fixture refused")
    });
    assert_eq!(result, Err("fixture refused"));
    assert_eq!(custom_stat_declarations(), ["fixture:new"]);
    let panic = std::panic::catch_unwind(|| {
        let _: Result<(), ()> = rebuild_parts(|| {
            install("fixture:panicked");
            panic!("fixture refused");
        });
    });
    assert!(panic.is_err());
    assert_eq!(custom_stat_declarations(), ["fixture:new"]);
}

/// Capture this thread's test installation for work sent to another thread.
///
/// Production installations are process-wide. Tests isolate them per thread; a worker
/// using an owning thread's immutable resource graph must see that same installation.
pub fn inherit_mods<A, R, F: FnOnce(&A) -> R + Send>(f: F) -> impl FnOnce(&A) -> R + Send {
    let installed = crate::runtime::installed();
    let parts = crate::parts::installed();
    move |argument| {
        struct Restore(
            Option<&'static Installed>,
            Option<&'static crate::parts::Installed>,
        );
        impl Drop for Restore {
            fn drop(&mut self) {
                TEST_MODS.set(self.0);
                crate::parts::TEST_PARTS.set(self.1);
            }
        }
        let _restore = Restore(
            TEST_MODS.replace(installed),
            crate::parts::TEST_PARTS.replace(parts),
        );
        f(argument)
    }
}

struct RestoreControllers(Option<&'static [ControllerInfo]>);
impl Drop for RestoreControllers {
    fn drop(&mut self) {
        TEST_CONTROLLERS.set(self.0);
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// A server and client host for tests, composed of one fake per service: the registries, the
/// players, one level, the commands and messages services, and the transaction journal, which
/// keeps the players and the level on each transaction and puts them back when it aborts.
///
/// Set a test's world up through the fakes' fields, run the code under test with
/// `&mut host` as its `&mut dyn ServerHost`, then read the fakes back. A host method no fake
/// answers panics, naming the method, as a test double's would. The fake does not lend a
/// level random stream, so [`LevelRef::with_random`] returns `None`. The host has no client
/// backend, so client handle writes refuse.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeHost {
    /// The registries.
    pub registries: FakeRegistries,
    /// The players online.
    pub players: FakePlayers,
    /// The one level.
    pub level: FakeLevel,
    /// The command service.
    pub commands: FakeCommands,
    /// The message service.
    pub messages: FakeMessages,
    /// The transaction journal over the players, the level and the holders' attachments.
    pub journal: FakeJournal<(
        FakePlayers,
        FakeLevel,
        sunrise_mod_runtime::attachments::Attachments,
    )>,
    /// The sounds played in the level, in order. Like the game's, they are sent as they are
    /// played: an aborted transaction does not take them back.
    pub sounds: Vec<FakeSound>,
    /// The game events sent in the level, in order, kept as the sounds are.
    pub game_events: Vec<FakeGameEvent>,
    /// The holders' attachments: a lone map answers for any holder.
    pub attachments: sunrise_mod_runtime::attachments::Attachments,
    scratch: scratch::FakeScratch,
}

impl FakeHost {
    /// A host with an empty overworld, no players and no mods loaded.
    pub fn new() -> Self {
        Self::default()
    }
    /// Freeze the current load's entity-type table after declaring the fake world's entities.
    /// Includes requested mod keys, the player type and every kind in this host's level.
    /// Repeated setup may use already resolved kinds, but cannot extend the frozen table.
    pub fn resolve_entity_types(&mut self) -> Result<(), Vec<String>> {
        if crate::parts::installed().is_none() {
            crate::parts::install_input(
                crate::__private::parts_installation::InstalledInput::default(),
            );
        }
        let mut ids: Vec<_> = crate::parts::requested_keys()
            .entity_types
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        ids.push("minecraft:player");
        ids.extend(
            self.level
                .entities
                .values()
                .map(|entity| entity.kind.as_str()),
        );
        if crate::parts::installed()
            .is_some_and(|installed| installed.resolved_entity_types.get().is_some())
        {
            let errors: Vec<_> = ids
                .into_iter()
                .filter(|id| crate::content::registry_keys::resolved_entity_type_key(id).is_none())
                .map(|id| {
                    format!("fake entity kind {id} was not declared before entity-type resolution")
                })
                .collect();
            return if errors.is_empty() {
                Ok(())
            } else {
                Err(errors)
            };
        }
        crate::content::registry_keys::resolve_entity_types(ids, |_| None)
    }
    /// Player `uuid` online as entity `entity` in the level, joined if missing.
    pub fn join(&mut self, uuid: u128, entity: u32) -> &mut FakePlayer {
        self.players.join(uuid, entity, self.level.level())
    }

    /// A queued bucket sound and game event, as the game plays them at the outermost commit.
    fn play_fluid_effects(&mut self, queued: level::QueuedFluidEffects) {
        if let Some(sound) = self
            .registries
            .fluid_sound_named(queued.fluid, queued.pickup)
        {
            self.sounds.push(FakeSound {
                sound: sound.to_owned(),
                at: queued.at,
                source: crate::SoundSource::Blocks,
                volume: 1.0,
                pitch: 1.0,
            });
        }
        self.game_events.push(FakeGameEvent {
            event: match queued.pickup {
                true => "minecraft:fluid_pickup",
                false => "minecraft:fluid_place",
            }
            .to_owned(),
            at: queued.at,
            context: crate::GameEventContext {
                source_entity: queued.player.map(PlayerRef::entity),
                affected_state: None,
            },
        });
    }
    fn container_slot(&self, at: BlockSide, slot: usize) -> Option<&ItemStack> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        items.slots.get(slot)?.as_ref()
    }
    fn max_stack(&self, resource: &crate::ItemResource) -> u32 {
        match resource.value_id(self, "minecraft:max_stack_size") {
            Some(Tag::Int(size)) if (1..=99).contains(&size) => size as u32,
            None => 1,
            _ => 0,
        }
    }
    fn validate_scratch_stack(&self, stack: &ItemStack) -> Result<(), StackWriteError> {
        if FakeRegistries::resolved_item_id(stack.item()).is_none() {
            return Err(StackWriteError::UnknownItem);
        }
        let maximum = self.max_stack(&crate::ItemResource::of(stack));
        if maximum == 0 {
            return Err(StackWriteError::InvalidMaximum);
        }
        Ok(())
    }
    fn run_transaction_scope(
        &mut self,
        loan: Option<(Option<ItemKey>, ItemStack)>,
        run: impl for<'loan> FnOnce(
            &'loan mut FakeHost,
            &'loan TransactionContext,
            Option<ScratchSlot>,
        ) -> bool,
    ) -> Result<(bool, Option<scratch::FakeScratchEntry>), TransactionScopeError> {
        struct ScratchScopeGuard<'a> {
            host: &'a mut FakeHost,
            owned_id: Option<u64>,
            closed: bool,
        }
        impl ScratchScopeGuard<'_> {
            fn finish(mut self, commit: bool) -> Option<scratch::FakeScratchEntry> {
                self.closed = true;
                let entry = self
                    .owned_id
                    .and_then(|id| self.host.scratch.remove_loan(id));
                self.host.scratch.close_frame(commit);
                if commit { entry } else { None }
            }
        }
        impl Drop for ScratchScopeGuard<'_> {
            fn drop(&mut self) {
                if !self.closed {
                    if let Some(id) = self.owned_id {
                        self.host.scratch.remove_loan(id);
                    }
                    self.host.scratch.close_frame(false);
                }
            }
        }

        self.scratch.push_frame();
        let slot = loan.map(|(item, stack)| self.scratch.insert_loan(item, stack));
        let guard = ScratchScopeGuard {
            host: self,
            owned_id: slot.map(ScratchSlot::id),
            closed: false,
        };
        let committed = FakeJournal::with_scope_with(
            &mut *guard.host,
            |host| &mut host.journal,
            |host| {
                (
                    host.players.clone(),
                    host.level.clone(),
                    host.attachments.clone(),
                )
            },
            |host, (players, level, attachments)| {
                (host.players, host.level, host.attachments) = (players, level, attachments);
            },
            |host, tx| run(host, tx, slot),
        )?;
        let published = guard.finish(committed);
        // The outermost commit plays what waited for it; an abort already restored `level`.
        if self.journal.depth() == 0 {
            for queued in std::mem::take(&mut self.level.fluid_effects) {
                self.play_fluid_effects(queued);
            }
        }
        Ok((committed, published))
    }
}

#[cfg(test)]
impl FakeHost {
    pub(crate) fn scratch_live_count(&self) -> usize {
        self.scratch.live_count()
    }
    pub(crate) fn scratch_kept_frame_counts(&self) -> Vec<usize> {
        self.scratch.kept_frame_counts()
    }
    pub(crate) fn scratch_kept_contains(&self, frame_index: usize, slot: ScratchSlot) -> bool {
        self.scratch.kept_contains(frame_index, slot)
    }
    pub(crate) fn with_test_transaction<R>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut FakeHost, &'loan TransactionContext) -> (bool, R),
    ) -> R {
        let mut out = None;
        self.run_transaction_scope(None, |host, tx, _| {
            let (commit, value) = f(host, tx);
            out = Some(value);
            commit
        })
        .expect("FakeHost transactions are always available");
        out.expect("transaction callback ran")
    }
    pub(crate) fn with_test_scratch_stack<R>(
        &mut self,
        stack: ItemStack,
        f: impl for<'loan> FnOnce(
            &'loan mut FakeHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> (bool, R),
    ) -> (Option<Option<ItemStack>>, R) {
        self.validate_scratch_stack(&stack)
            .expect("test scratch stack is valid");
        let item = stack.item();
        let mut out = None;
        let (_, published) = self
            .run_transaction_scope(Some((Some(item), stack)), |host, tx, slot| {
                let place = ItemPlace::Scratch(slot.expect("scratch loan minted"));
                let (commit, value) = f(host, tx, &place);
                out = Some(value);
                commit
            })
            .expect("FakeHost transactions are always available");
        (
            published.map(|entry| entry.stack),
            out.expect("scratch callback ran"),
        )
    }
}
impl Host for FakeHost {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
impl crate::prelude::BlockEntitiesHost for FakeHost {
    fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        self.level.standing(level, pos)?;
        Some(BlockEntityRef {
            level,
            position: pos,
            generation: 0,
        })
    }
}
impl crate::prelude::EntitiesHost for FakeHost {
    fn level(&self, dimension: &str) -> Option<LevelRef> {
        let level = self.level.level();
        (crate::content::registry_keys::dimension_id(level.dimension()) == Some(dimension))
            .then_some(level)
    }
    fn entity(&self, id: u32) -> Option<EntityRef> {
        let known = self.players.by_entity(id).is_some() || self.level.entity_at(id).is_some();
        known.then(|| EntityRef::new(WorldEpoch::new(FAKE_EPOCH), id))
    }
}
impl crate::prelude::PlayersHost for FakeHost {
    fn player(&self, uuid: u128) -> Option<PlayerRef> {
        self.players.handle(uuid)
    }
}
impl crate::prelude::ItemsHost for FakeHost {
    fn item_key(&self, id: &crate::Identifier) -> Option<crate::ItemKey> {
        crate::content::registry_keys::item_key_by_identifier(id)
    }
    fn item_default_component(&self, item: crate::ItemKey, component: &str) -> Option<Tag> {
        self.registries
            .default_component(FakeRegistries::resolved_item_id(item)?, component)
    }
    fn max_stack_size(&self, item: crate::ItemKey) -> Option<u32> {
        Some(
            self.registries
                .max_stack_size(FakeRegistries::resolved_item_id(item)?),
        )
    }
    fn max_damage(&self, item: crate::ItemKey) -> Option<u32> {
        let id = FakeRegistries::resolved_item_id(item)?;
        match self
            .registries
            .default_component(id, "minecraft:max_damage")
        {
            Some(Tag::Int(damage)) => u32::try_from(damage).ok(),
            None => Some(0),
            _ => None,
        }
    }
}
impl crate::prelude::TagsHost for FakeHost {
    fn tags(&self) -> &dyn crate::Tags {
        &self.registries
    }
}
impl crate::prelude::BlockStatesHost for FakeHost {
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey> {
        crate::content::registry_keys::fluid_key(id)
    }
}
impl crate::prelude::LevelsHost for FakeHost {
    fn is_loaded(&self, id: &str) -> bool {
        self.registries.is_loaded(id)
    }
}
crate::impl_host_areas!(
    FakeHost,
    except(
        block_entities,
        block_states,
        entities,
        items,
        levels,
        mod_state,
        players,
        tags
    )
);

impl ServerHost for FakeHost {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(BackendMutRef::of(self))
    }
    fn with_current_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext),
    ) -> Result<(), crate::TransactionUnavailable> {
        FakeJournal::with_current_scope(self, |host| &host.journal, callback)
    }

    fn with_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
        ) -> bool,
    ) -> Result<bool, TransactionScopeError> {
        self.run_transaction_scope(None, |host, tx, _| callback(host, tx))
            .map(|(committed, _)| committed)
    }
    fn with_scratch_stack_scope(
        &mut self,
        stack: ItemStack,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> bool,
    ) -> Result<Option<Option<ItemStack>>, ScratchScopeError> {
        self.validate_scratch_stack(&stack)
            .map_err(ScratchScopeError::Stack)?;
        let item = stack.item();
        let (_, published) = self
            .run_transaction_scope(Some((Some(item), stack)), |host, tx, slot| {
                let Some(slot) = slot else { return false };
                let place = ItemPlace::Scratch(slot);
                callback(host, tx, &place)
            })
            .map_err(ScratchScopeError::Transaction)?;
        Ok(published.map(|entry| entry.stack))
    }
    fn with_scratch_slots_scope(
        &mut self,
        stack: ItemStack,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> bool,
    ) -> Result<Option<(Option<ItemStack>, Option<ItemStack>)>, ScratchScopeError> {
        self.validate_scratch_stack(&stack)
            .map_err(ScratchScopeError::Stack)?;
        let (_, published) = self
            .run_transaction_scope(Some((None, stack)), |host, tx, slot| {
                let Some(slot) = slot else { return false };
                let place = ItemPlace::Scratch(slot);
                callback(host, tx, &place)
            })
            .map_err(ScratchScopeError::Transaction)?;
        Ok(published.map(|entry| (entry.stack, entry.overflow)))
    }
    fn transaction_matches(&self, tx: Option<&TransactionContext>) -> bool {
        self.journal.matches(tx)
    }
}
impl crate::host::mod_state::ModStateHost for FakeHost {
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        &mut self.attachments
    }
}
impl crate::prelude::ModStateServerHost for FakeHost {
    fn transaction_lifecycle(&self) -> crate::TransactionLifecycle {
        // The fake tells the world of nothing at a close, so no close is ever seen running.
        if self.journal.depth() == 0 {
            crate::TransactionLifecycle::None
        } else {
            crate::TransactionLifecycle::Open
        }
    }
}
impl crate::prelude::ServicesServerHost for FakeHost {
    fn commands(&mut self) -> &mut dyn Commands {
        &mut self.commands
    }
    fn messages(&self) -> &dyn Messages {
        &self.messages
    }
    fn players(&self) -> &dyn Players {
        &self.players
    }
}
crate::impl_server_host_areas!(FakeHost, except(mod_state, services));
impl ClientHost for FakeHost {}
crate::impl_client_host_areas!(FakeHost);

impl Backend for FakeHost {}
impl crate::LevelHost for FakeHost {
    fn block_entity_data(&self, _: BlockEntityRef) -> Option<crate::Compound> {
        None
    }
    fn block_entity(&self, pos: [i32; 3]) -> Option<BlockEntityRef> {
        crate::prelude::BlockEntitiesHost::block_entity(self, self.level.level(), pos)
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<crate::BlockState> {
        let name = self
            .level
            .block_at(self.level.level(), pos)
            .unwrap_or("minecraft:air");
        let block = crate::__private::block_key(name).or_else(|| {
            crate::parts::requested_keys()
                .blocks
                .slots
                .get(name)
                .copied()
                .map(crate::BlockKey)
        });
        // The stand-in can hold a container without a published block registry. That
        // unnamed state never selects a block-keyed provider.
        Some(crate::block::block_state::block_state_from_ordinal(
            block.unwrap_or(crate::BlockKey(u32::MAX)),
            0,
        ))
    }
    fn has_item_capability(&self, pos: [i32; 3], side: Option<crate::Direction>) -> Option<bool> {
        let at = BlockSide {
            level: self.level.level(),
            pos,
            side,
        };
        let mut answer = None;
        crate::prelude::LevelBlocksBackend::level_with_capability_site(
            self,
            at,
            &mut |view, site| {
                answer = Some(crate::__private::item_capability_present(view, site));
            },
        )
        .ok()?;
        answer
    }
}
impl crate::prelude::LevelBlocksBackend for FakeHost {
    fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<crate::BlockKey> {
        self.level
            .block_at(level, pos)
            .and_then(crate::__private::block_key)
    }
    fn level_with_capability_site(
        &self,
        at: BlockSide,
        visit: &mut dyn FnMut(&dyn crate::LevelHost, crate::BlockCapabilitySite),
    ) -> Result<(), crate::LevelReadError> {
        if at.level != self.level.level() {
            return Err(crate::LevelReadError::Unavailable);
        }
        let Some(state) = crate::LevelHost::block_state(self, at.pos) else {
            return Err(crate::LevelReadError::Unavailable);
        };
        let resident =
            crate::prelude::BlockEntitiesHost::block_entity(self, at.level, at.pos).map(|entity| {
                (
                    entity,
                    crate::prelude::BlockEntitiesBackend::block_entity_kind(self, entity),
                )
            });
        let builtin_items = crate::prelude::ItemsBackend::container_size(self, at).is_some();
        visit(
            self,
            crate::BlockCapabilitySite::new(at, state, resident, builtin_items),
        );
        Ok(())
    }
    /// The named block's first state: the fake keeps no properties.
    fn level_block_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<crate::BlockState> {
        let block = crate::prelude::LevelBlocksBackend::level_block(self, level, pos)?;
        Some(crate::__private::block_state_from_ordinal(block, 0))
    }
}
impl crate::prelude::LevelTickingBackend for FakeHost {
    fn level_in_place(&self, level: LevelRef) -> bool {
        level == self.level.level()
    }
}
impl crate::prelude::BlockEntitiesBackend for FakeHost {
    fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey> {
        let standing = self.level.standing(entity.level, entity.position)?;
        standing.kind().cloned()
    }
}
impl crate::prelude::ItemsBackend for FakeHost {
    fn container_kind_amount(&self, at: BlockSide, resource: &crate::ItemResource) -> Option<u64> {
        let Some(items) = self
            .level
            .standing(at.level, at.pos)
            .and_then(|s| s.items.as_ref())
        else {
            return Some(0);
        };
        items
            .slots
            .iter()
            .flatten()
            .filter(|stack| resource.matches(stack))
            .try_fold(0u64, |total, stack| total.checked_add(stack.count() as u64))
    }
    fn container_kind_space(&self, at: BlockSide, resource: &crate::ItemResource) -> Option<u64> {
        let max_stack = self.max_stack(resource);
        let Some(items) = self
            .level
            .standing(at.level, at.pos)
            .and_then(|s| s.items.as_ref())
        else {
            return Some(0);
        };
        (0..items.slots.len()).try_fold(0u64, |total, slot| {
            let room = match &items.slots[slot] {
                Some(held) if resource.matches(held) => {
                    items.capacity(slot, max_stack).saturating_sub(held.count())
                }
                Some(_) => 0,
                None => items.capacity(slot, max_stack),
            };
            total.checked_add(room as u64)
        })
    }
    fn container_visit_entries(
        &self,
        at: BlockSide,
        visitor: &mut dyn FnMut(&crate::ItemResource, u64) -> std::ops::ControlFlow<()>,
    ) -> Result<(), crate::LevelReadError> {
        let Some(items) = self
            .level
            .standing(at.level, at.pos)
            .and_then(|s| s.items.as_ref())
        else {
            return Ok(());
        };
        for stack in items.slots.iter().flatten() {
            if visitor(&crate::ItemResource::of(stack), u64::from(stack.count())).is_break() {
                break;
            }
        }
        Ok(())
    }
    fn container_size(&self, at: BlockSide) -> Option<usize> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        Some(items.slots.len())
    }
    fn lend_container_stack(
        &self,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) {
        if let Some(stack) = self.container_slot(at, slot) {
            read(ItemStackView::from_stack(stack));
        }
    }
    fn container_slot_backing(&self, at: BlockSide, slot: usize) -> Option<crate::StorageSlot> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        (slot < items.slots.len()).then_some(crate::StorageSlot {
            level: at.level,
            pos: at.pos,
            slot,
        })
    }
    fn container_capacity(&self, at: BlockSide, slot: usize, stack: Option<&ItemStack>) -> u32 {
        let items = self
            .level
            .standing(at.level, at.pos)
            .and_then(|standing| standing.items.as_ref());
        items.map_or(0, |items| {
            items.capacity(
                slot,
                stack.map_or(u32::MAX, |stack| {
                    self.max_stack(&crate::ItemResource::of(stack))
                }),
            )
        })
    }
    fn entity_container_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn entity_automation_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn entity_equipment_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn bundle_slots(
        &self,
        _: &ItemStack,
        _: &crate::BundleConfig,
    ) -> Result<Option<Vec<Option<ItemStack>>>, crate::BundleReadError> {
        // The fake registry models declared containers, not game bundle components.
        Ok(None)
    }
    fn item_container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
        self.registries.container_slots(stack)
    }
}
impl crate::prelude::FluidsEnergyBackend for FakeHost {
    fn fluid_tank_count(&self, at: BlockSide) -> Option<usize> {
        Some(self.level.standing(at.level, at.pos)?.tanks.len())
    }
    fn fluid_tank(&self, at: BlockSide, tank: usize) -> Option<FluidStack> {
        let standing = self.level.standing(at.level, at.pos)?;
        standing.tanks.get(tank)?.held.clone()
    }
    fn fluid_tank_capacity(&self, at: BlockSide, tank: usize, _: Option<&FluidStack>) -> u32 {
        let standing = self.level.standing(at.level, at.pos);
        standing
            .and_then(|standing| standing.tanks.get(tank))
            .map_or(0, |tank| tank.capacity)
    }
    fn energy_amount(&self, at: BlockSide) -> Option<u32> {
        let energy = self.level.standing(at.level, at.pos)?.energy.as_ref()?;
        Some(energy.amount)
    }
    fn energy_capacity(&self, at: BlockSide) -> u32 {
        let standing = self.level.standing(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_ref())
            .map_or(0, |energy| energy.capacity)
    }
}
impl crate::prelude::EntityIdentityBackend for FakeHost {
    fn entity_kind(&self, entity: EntityRef) -> Option<crate::EntityTypeKey> {
        let kind = if self.players.by_entity(entity.id()).is_some() {
            "minecraft:player"
        } else {
            &self.level.entity_at(entity.id())?.kind
        };
        crate::content::registry_keys::resolved_entity_type_key(kind)
    }
    fn entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        if let Some((_, player)) = self.players.by_entity(entity.id()) {
            return Some(player.level);
        }
        Some(self.level.entity_at(entity.id())?.level)
    }
}
impl crate::prelude::KeysBackend for FakeHost {
    fn entity_type_key_id(&self, key: crate::EntityTypeKey) -> Option<crate::RegistryName> {
        crate::content::registry_keys::entity_type_key_id(key)
    }

    fn item_key_id(&self, key: crate::ItemKey) -> Option<&str> {
        crate::content::registry_keys::item_id(key)
    }
}
impl crate::prelude::PlayerInventoryBackend for FakeHost {
    fn player_selected_slot(&self, player: PlayerRef) -> Option<usize> {
        Some(self.players.get(player.uuid())?.selected_slot)
    }
    fn player_inventory_slots(
        &self,
        player: PlayerRef,
        read: &mut dyn for<'a> FnMut(InventoryView<'a>),
    ) -> Option<()> {
        let slots = &self.players.get(player.uuid())?.slots;
        let stack_at = |slot: u32| {
            let index = match slot {
                0..=35 => slot as i32,
                36..=39 => slot as i32 + 64,
                40 => -106,
                // The fake stores ItemPlace slots only.
                _ => return None,
            };
            slots.get(&index)
        };
        read(InventoryView::new(
            0..43,
            &|slot| Some(stack_at(slot)?.read_metadata()),
            &|slot, _| stack_at(slot).cloned(),
        ));
        Some(())
    }
}
impl crate::prelude::PlayerModesBackend for FakeHost {
    fn player_has_infinite_materials(&self, player: PlayerRef) -> Option<bool> {
        Some(self.players.get(player.uuid())?.infinite_materials)
    }
}
impl crate::prelude::PlayerSessionBackend for FakeHost {
    fn player_name(&self, player: PlayerRef) -> Option<&str> {
        Some(&self.players.get(player.uuid())?.name)
    }
}
impl crate::prelude::PlayerWorldBackend for FakeHost {
    fn player_level(&self, player: PlayerRef) -> Option<LevelRef> {
        (self.players.handle(player.uuid())? == player).then_some(())?;
        Some(self.players.get(player.uuid())?.level)
    }
    fn player_dimension(&self, player: PlayerRef) -> Option<crate::Identifier> {
        let level = self.player_level(player)?;
        crate::Identifier::parse(level.dimension_id()).ok()
    }
}
crate::block::block_states::impl_block_states_without_tables!(FakeHost {
    fn fluid_bucket(
        &self,
        fluid: crate::FluidKey,
    ) -> Result<Option<crate::ItemKey>, crate::LevelReadError> {
        Ok(self.registries.bucket_of(fluid))
    }
    fn bucket_fluid(
        &self,
        bucket: crate::ItemKey,
    ) -> Result<Option<crate::FluidKey>, crate::LevelReadError> {
        Ok(self.registries.fluid_of(bucket))
    }
    fn fluid_pickup_sound(
        &self,
        fluid: crate::FluidKey,
    ) -> Result<Option<&'static str>, crate::LevelReadError> {
        Ok(self.registries.fluid_sound(fluid, true))
    }
    fn fluid_empty_sound(
        &self,
        fluid: crate::FluidKey,
    ) -> Result<Option<&'static str>, crate::LevelReadError> {
        Ok(self.registries.fluid_sound(fluid, false))
    }
});
crate::impl_backend_areas!(
    FakeHost,
    except(
        block_entities,
        block_states,
        entity_identity,
        fluids_energy,
        items,
        keys,
        level_blocks,
        level_ticking,
        player_inventory,
        player_modes,
        player_session,
        player_world
    )
);

impl BackendMut for FakeHost {}
impl crate::prelude::LevelTickingBackendMut for FakeHost {
    fn with_level_random(&mut self, _: LevelRef, _: crate::RandomDraw<'_>) {}
}
impl crate::prelude::ItemsBackendMut for FakeHost {
    fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &crate::ItemResource,
        amount: u32,
        tx: &TransactionContext,
    ) -> Result<Option<ItemStack>, crate::StackWriteError> {
        if !self.journal.matches(Some(tx)) {
            return Ok(resource.to_stack(amount));
        }
        let max_stack = self.max_stack(resource);
        if max_stack == 0 {
            return Err(crate::StackWriteError::InvalidMaximum);
        }
        let standing = self.level.standing_mut(at.level, at.pos);
        let put = match standing.and_then(|standing| standing.items.as_mut()) {
            Some(items) => items.insert(slot, resource, amount, max_stack),
            None => 0,
        };
        Ok(resource.to_stack(amount - put))
    }
    fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        tx: &TransactionContext,
    ) -> Option<ItemStack> {
        if !self.journal.matches(Some(tx)) {
            return None;
        }
        let standing = self.level.standing_mut(at.level, at.pos)?;
        standing.items.as_mut()?.extract(slot, max)
    }
    fn container_insert_kind(
        &mut self,
        at: BlockSide,
        resource: &crate::ItemResource,
        amount: u64,
        tx: &TransactionContext,
    ) -> u64 {
        use crate::prelude::ItemsBackend;
        if !self.journal.matches(Some(tx)) {
            return 0;
        }
        let size = self.container_size(at).unwrap_or(0);
        let mut left = amount;
        for holding in [true, false] {
            for slot in 0..size {
                if left == 0 {
                    break;
                }
                let pick = match self.container_slot(at, slot) {
                    Some(held) => holding && resource.matches(held),
                    None => !holding,
                };
                if !pick {
                    continue;
                }
                let ask = left.min(u32::MAX as u64) as u32;
                let rest = self
                    .container_insert(at, slot, resource, ask, tx)
                    .ok()
                    .flatten()
                    .map_or(0, |rest| rest.count());
                left -= (ask - rest) as u64;
            }
        }
        amount - left
    }
    fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &crate::ItemResource,
        amount: u64,
        tx: &TransactionContext,
    ) -> u64 {
        use crate::prelude::ItemsBackend;
        if !self.journal.matches(Some(tx)) {
            return 0;
        }
        let mut left = amount;
        for slot in 0..self.container_size(at).unwrap_or(0) {
            if left == 0 {
                break;
            }
            if self
                .container_slot(at, slot)
                .is_some_and(|held| resource.matches(held))
            {
                let ask = left.min(u32::MAX as u64) as u32;
                left -= self
                    .container_extract(at, slot, ask, tx)
                    .map_or(0, |out| out.count()) as u64;
            }
        }
        amount - left
    }
    fn game_stack(
        &mut self,
        place: &ItemPlace,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        if let ItemPlace::Scratch(slot) = place {
            let stack = self.scratch.held(*slot)?;
            read(ItemStackView::from_stack(stack));
            return Some(());
        }
        let (player, slot) = self.players.place(place)?;
        read(ItemStackView::from_stack(player.slots.get(&slot)?));
        Some(())
    }
    fn game_edit_stack(
        &mut self,
        place: &ItemPlace,
        tx: &TransactionContext,
        edit: &mut dyn for<'a> FnMut(&mut ItemComponentEditor<'a>),
    ) -> Result<(), ItemPlaceEditError> {
        if !self.journal.matches(Some(tx)) {
            return Err(ItemPlaceEditError::InvalidContext);
        }
        let (player, slot) = self
            .players
            .place(place)
            .ok_or(ItemPlaceEditError::NotHeld)?;
        let held = player
            .slots
            .get_mut(&slot)
            .ok_or(ItemPlaceEditError::NotHeld)?;
        let read_override = |id: &str| held.component_value(id).cloned();
        let edits = edit_item_components(ItemStackView::from_stack(held), &read_override, edit);
        edits.apply_to(held);
        Ok(())
    }
    fn game_on_crafted_by(
        &mut self,
        _: &ItemPlace,
        tx: &TransactionContext,
        _: i32,
    ) -> Result<(), crate::CraftedByError> {
        if !self.journal.matches(Some(tx)) {
            return Err(crate::CraftedByError::InvalidContext);
        }
        Err(crate::CraftedByError::Unavailable)
    }
    fn game_insert(
        &mut self,
        place: &ItemPlace,
        resource: &crate::ItemResource,
        amount: u32,
        tx: &TransactionContext,
    ) -> Result<u32, crate::StackWriteError> {
        if !self.journal.matches(Some(tx)) {
            return Ok(0);
        }
        let max_stack = self.max_stack(resource);
        if max_stack == 0 {
            return Err(crate::StackWriteError::InvalidMaximum);
        }
        if let ItemPlace::Scratch(slot) = place {
            return Ok(self.scratch.insert(*slot, resource, amount, max_stack));
        }
        Ok(match self.players.place(place) {
            Some((player, slot)) => player.insert(slot, resource, amount, max_stack),
            _ => 0,
        })
    }
    fn game_extract(
        &mut self,
        place: &ItemPlace,
        resource: &crate::ItemResource,
        amount: u32,
        tx: &TransactionContext,
    ) -> Result<u32, crate::StackWriteError> {
        if !self.journal.matches(Some(tx)) {
            return Ok(0);
        }
        if let ItemPlace::Scratch(slot) = place {
            return Ok(self.scratch.extract(*slot, resource, amount));
        }
        Ok(self
            .players
            .place(place)
            .map_or(0, |(player, slot)| player.extract(slot, resource, amount)))
    }
}
impl crate::prelude::FluidsEnergyBackendMut for FakeHost {
    fn fluid_tank_insert(
        &mut self,
        at: BlockSide,
        tank: usize,
        fluid: FluidStack,
        tx: &TransactionContext,
    ) -> Option<FluidStack> {
        if !self.journal.matches(Some(tx)) {
            return Some(fluid);
        }
        let standing = self.level.standing_mut(at.level, at.pos);
        match standing.and_then(|standing| standing.tanks.get_mut(tank)) {
            Some(tank) => tank.insert(fluid),
            None => Some(fluid),
        }
    }
    fn fluid_tank_extract(
        &mut self,
        at: BlockSide,
        tank: usize,
        max: u32,
        tx: &TransactionContext,
    ) -> Option<FluidStack> {
        if !self.journal.matches(Some(tx)) {
            return None;
        }
        let standing = self.level.standing_mut(at.level, at.pos)?;
        standing.tanks.get_mut(tank)?.extract(max)
    }
    fn energy_insert(&mut self, at: BlockSide, amount: u32, tx: &TransactionContext) -> u32 {
        if !self.journal.matches(Some(tx)) {
            return 0;
        }
        let standing = self.level.standing_mut(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_mut())
            .map_or(0, |energy| energy.insert(amount))
    }
    fn energy_extract(&mut self, at: BlockSide, amount: u32, tx: &TransactionContext) -> u32 {
        if !self.journal.matches(Some(tx)) {
            return 0;
        }
        let standing = self.level.standing_mut(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_mut())
            .map_or(0, |energy| energy.extract(amount))
    }
    /// The source block at `pos` ([`FakeLevel::set_block`]), a bucket of it; the fake plays no
    /// sound or game event for it.
    fn level_pickup_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        _: Option<PlayerRef>,
        tx: &TransactionContext,
    ) -> Result<Option<FluidStack>, crate::TransactionError<()>> {
        if !self.journal.matches(Some(tx)) {
            return Err(crate::TransactionError::InvalidContext);
        }
        Ok(self
            .level
            .take_source(level, pos)
            .map(|fluid| FluidStack::new(fluid, 1000)))
    }
    /// A source of `fluid` where nothing is; the fake plays no sound or game event for it.
    fn level_place_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: crate::FluidKey,
        _: Option<PlayerRef>,
        _: bool,
        tx: &TransactionContext,
    ) -> Result<bool, crate::TransactionError<()>> {
        if !self.journal.matches(Some(tx)) {
            return Err(crate::TransactionError::InvalidContext);
        }
        Ok(self.level.put_source(level, pos, fluid))
    }
    /// Queued with the level for the outermost commit, which plays it as the game does.
    fn level_play_fluid_effects(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: crate::FluidKey,
        pickup: bool,
        player: Option<PlayerRef>,
        tx: &TransactionContext,
    ) -> Result<bool, crate::TransactionError<()>> {
        if !self.journal.matches(Some(tx)) {
            return Err(crate::TransactionError::InvalidContext);
        }
        let Some(id) = crate::content::registry_keys::fluid_id(fluid) else {
            return Ok(false);
        };
        if level != self.level.level() {
            return Ok(false);
        }
        self.level.fluid_effects.push(level::QueuedFluidEffects {
            at: pos.map(|c| f64::from(c) + 0.5),
            fluid: id,
            pickup,
            player,
        });
        Ok(true)
    }
}
impl crate::prelude::EntityIdentityBackendMut for FakeHost {
    fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
        if let Some((uuid, _)) = self.players.by_entity(entity.id()) {
            return Some(uuid);
        }
        Some(self.level.entity_at(entity.id())?.uuid)
    }
}
impl crate::prelude::PlayerMessagesBackendMut for FakeHost {
    fn player_action_bar(
        &mut self,
        player: PlayerRef,
        line: Text,
    ) -> Result<bool, crate::text::TextFormError> {
        line.validate_depth()?;
        let Some(player) = self.players.get_mut(player.uuid()) else {
            return Ok(false);
        };
        player.action_bar.push(line);
        Ok(true)
    }
}
impl crate::prelude::LevelEffectsBackendMut for FakeHost {
    fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: crate::SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
    ) -> bool {
        if level != self.level.level()
            || except.is_some_and(|p| self.players.handle(p.uuid()) != Some(p))
        {
            return false;
        }
        self.sounds.push(FakeSound {
            sound: sound.to_owned(),
            at,
            source,
            volume,
            pitch,
        });
        true
    }
    fn level_game_event(
        &mut self,
        level: LevelRef,
        event: &str,
        at: [f64; 3],
        context: crate::GameEventContext,
    ) -> bool {
        if level != self.level.level() {
            return false;
        }
        self.game_events.push(FakeGameEvent {
            event: event.to_owned(),
            at,
            context,
        });
        true
    }
}
crate::impl_backend_mut_areas!(
    FakeHost,
    except(
        entity_identity,
        fluids_energy,
        items,
        level_effects,
        level_ticking,
        player_messages
    )
);

/// Use the game fixture's completed Parts registration in place of its earlier probe.
// hidden: only the game test fixture scopes the controller table after loading Parts.
#[doc(hidden)]
pub fn with_installed_ticket_controllers<R>(f: impl FnOnce() -> R) -> R {
    let _controllers =
        RestoreControllers(TEST_CONTROLLERS.replace(Some(crate::parts::installed_controllers())));
    f()
}

/// Scope the controller lookup rows prepared by the API-owned registration fixture.
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
pub fn with_controller_tables<R>(
    controllers: Vec<crate::__private::parts_installation::ControllerInfoInput>,
    poses: Vec<crate::__private::parts_installation::PoseInfoInput>,
    f: impl FnOnce() -> R,
) -> R {
    let controllers: Vec<ControllerInfo> = controllers.into_iter().map(Into::into).collect();
    let controllers: &'static [_] = Box::leak(controllers.into_boxed_slice());
    let _poses = crate::client::arm_pose::test_table(poses.into_iter().map(Into::into).collect());
    let _controllers = RestoreControllers(TEST_CONTROLLERS.replace(Some(controllers)));
    f()
}

// no wire: read-only in-process phase fixture metadata.
#[derive(Default)]
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
pub struct TestReader {
    pub registries: std::collections::BTreeMap<String, std::sync::Arc<[String]>>,
    /// `None` models a load without a recipe overlay; `Some` models its tag table.
    pub tags: Option<std::sync::Arc<[(String, std::sync::Arc<[String]>)]>>,
}
impl crate::LoadReader for TestReader {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.registries.get(name).map(AsRef::as_ref)
    }
    fn fluid_tags(&self) -> Option<&[(String, std::sync::Arc<[String]>)]> {
        self.tags.as_deref()
    }
    fn setting(&self, _: &str, _: &str) -> Option<&str> {
        None
    }
}
impl crate::LoopReader for TestReader {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.registries.get(name).map(AsRef::as_ref)
    }
    fn setting(&self, _: &str, _: &str) -> Option<&str> {
        None
    }
}

#[cfg(test)]
impl crate::__private::WorldgenReader for TestReader {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.registries.get(name).map(AsRef::as_ref)
    }
    fn setting(&self, _: &str, _: &str) -> Option<&str> {
        None
    }
}

/// Build a `(RegisteredAttachment, AttachmentKey<T>)` pair for tests without running full
/// mod registration.
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
pub fn registered_attachment_fixture<T: Send + Sync + 'static>(
    owner: &'static str,
    descriptor: crate::AttachmentDescriptor<T>,
) -> (crate::RegisteredAttachment, crate::AttachmentKey<T>) {
    crate::parts::registered_attachment_fixture(owner, descriptor)
}

/// Install a single attachment descriptor in the test thread's Parts table and return its live
/// [`AttachmentKey<T>`](crate::AttachmentKey).
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
pub fn install_attachment_fixture<T: Send + Sync + 'static>(
    owner: &'static str,
    descriptor: crate::AttachmentDescriptor<T>,
) -> crate::AttachmentKey<T> {
    crate::parts::install_attachment_fixture(owner, descriptor)
}

/// Retrieve the key issued by this test thread's registration fixture.
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
pub fn attachment_key<T: Send + Sync + 'static>(
    namespace: &str,
    name: &str,
) -> crate::AttachmentKey<T> {
    crate::parts::installed().unwrap().attachment_types[namespace][name]
        .downcast_key()
        .unwrap()
}

#[cfg(test)]
#[path = "testing/handle_level_tests.rs"]
mod handle_level_tests;
