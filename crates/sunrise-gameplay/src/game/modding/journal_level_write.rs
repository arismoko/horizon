//! The level in place during a transaction. Reads see tentative storage;
//! updates, scheduled ticks and effects are published only at the root commit.
use crate::game::ServerGame;
use mod_api::{
    AttachmentBatch, AttachmentStore, BlockEntityRef, BlockKey, BlockPos, BlockSide, BlockState,
    BlockUpdateFlags, Compound, Direction, FeatureFlags, FluidState, ItemStackView, LevelHost,
    LevelRandomHost, LevelRef, LevelTicksHost, LevelWriteHost, LightLayer, PartStateError,
    ParticleOptions, RandomDraw, SoundSource, Tags,
};
use sunrise_core::{random::RandomSource, ticks::TickPriority};
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
use sunrise_gameplay_common::game::creatures::NativeSound;
use sunrise_gameplay_common::game::journal_level_effects::PendingWrite;
use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;
use sunrise_gameplay_common::game::native_level_signals::ReadLevel;
use sunrise_gameplay_common::game::transactions::After;
use sunrise_sim::lan::server_effects::ServerQueuedParticle;
use sunrise_world::{
    fluid::FluidKind,
    level::{FluidWrite, LevelAccess as _, flags},
    registry::BlockStateId,
};

pub struct JournalLevelWrite<'g, 'a> {
    game: &'g mut crate::game::ServerGame<'a>,
    level: LevelRef,
}

impl<'g, 'a> JournalLevelWrite<'g, 'a> {
    pub fn new(game: &'g mut crate::game::ServerGame<'a>, level: LevelRef) -> Option<Self> {
        (game.level_in_place(level) && game.has_level_transaction()).then_some(Self { game, level })
    }

    fn read(&self) -> ReadLevel<'_, 'a, ServerGame<'a>> {
        ReadLevel::new(&*self.game, self.level, &self.game.world)
    }

    pub(in crate::game::server_side) fn write_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        let old = self.game.world.block(pos[0], pos[1], pos[2]);
        self.can_replace_block(pos, old, state) && self.replace_block(pos, old, state, flags)
    }

    fn can_replace_block(&self, pos: [i32; 3], old: BlockStateId, state: BlockStateId) -> bool {
        let [x, y, z] = pos;
        old != state
            && !self.game.server.world_debug
            && (-30_000_000..30_000_000).contains(&x)
            && (-30_000_000..30_000_000).contains(&z)
            && self.game.world.shape.contains_y(y)
            && self
                .game
                .world
                .chunks
                .get(&(x >> 4, z >> 4))
                .is_some_and(|chunk| chunk.section_index(y).is_some())
            && !self.game.world.carrier_fences.locks_cell(&pos)
            && self.game.pack.assets.registry.state(state).is_some()
    }

    fn replace_block(
        &mut self,
        pos: [i32; 3],
        old: BlockStateId,
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        if !self.game.journal_set_block_flags(pos, state, flags, true) {
            return false;
        }
        let [x, y, z] = pos;
        let key = (x, y, z);
        let replaces_entity = !self.game.res.behaviours().keeps_block_entity(old, state);
        if replaces_entity && self.game.world.block_entities.contains_key(&key) {
            self.game.journal_block_replacement(key);
        }
        if self.game.captures_placement() && !self.game.world.block_entities.contains_key(&key) {
            self.game.ensure_stateful_entity_table();
            let entity = self.game.with_placement_preview_level(|_, level, _| {
                level.new_block_entity_for_state(state)
            });
            self.game.journal_stage_block_entity(key, entity);
        } else if replaces_entity {
            self.game.ensure_stateful_entity_table();
            if !self.game.world.block_entities.stateful_entity(state.0)
                && self.game.res.facts().get(old).name != self.game.res.facts().get(state).name
                && let Some(entity) = self.game.new_block_entity_for_state(state)
            {
                self.game.journal_stage_block_entity(key, Some(entity));
            }
        }
        true
    }

    /// `Level.destroyBlock` in this level's journal. Resolve the loot and experience while
    /// the old state and block entity are still visible; publication follows vanilla order.
    pub(in crate::game::server_side) fn destroy_block(
        &mut self,
        pos: [i32; 3],
        drop: bool,
        tool: Option<&sunrise_items::items::ItemStack>,
        breaker: Option<mod_api::EntityRef>,
        collect: bool,
    ) -> Option<Vec<sunrise_items::items::ItemStack>> {
        let [x, y, z] = pos;
        let old = self.game.world.block(x, y, z);
        let behaviours = self.game.res.behaviours();
        let replacement = behaviours.legacy_block(behaviours.fluid(old));
        if behaviours.is_air(old) || !self.can_replace_block(pos, old, replacement) {
            return None;
        }
        if let Some(id) = behaviours.destroy_particle_event(old) {
            self.queue(PendingWrite::Event {
                level: self.game.dim,
                id,
                pos,
                data: old.0 as i32,
            });
        }
        let collected = if drop {
            self.drop_resources(pos, old, tool, breaker, collect)
        } else {
            Vec::new()
        };
        // Only storage (not listeners) is touched by this already-validated write.
        if !self.replace_block(pos, old, replacement, flags::UPDATE_ALL) {
            return None;
        }
        self.game.after_commit(After::DestroyBlockGameEvent {
            pos,
            state: old,
            breaker,
        });
        Some(collected)
    }

    /// `Block.dropResources` of `old` at `pos` inside the transaction, the block left as it
    /// is: the loot is rolled against the block now, and its item entities and experience
    /// spawn at commit. Nothing under `doTileDrops` false. With `collect`, the stacks come
    /// back instead of spawning.
    pub(in crate::game::server_side) fn drop_resources(
        &mut self,
        pos: [i32; 3],
        old: BlockStateId,
        tool: Option<&sunrise_items::items::ItemStack>,
        breaker: Option<mod_api::EntityRef>,
        collect: bool,
    ) -> Vec<sunrise_items::items::ItemStack> {
        let [x, y, z] = pos;
        let mut collected = Vec::new();
        if self.game.level_rules().block_drops {
            self.game.journal_block_loot_random();
            // First access to container loot can fill the inventory and consume its loot table.
            let key = (x, y, z);
            if self.game.world.block_entities.contains_key(&key)
                && self
                    .game
                    .world
                    .chunks
                    .get(&(x >> 4, z >> 4))
                    .and_then(|chunk| chunk.saved.as_ref())
                    .is_some_and(|saved| saved.pending_loot.contains_key(&key))
            {
                self.game.journal_attachments(
                    sunrise_mod_runtime::attachments::Holder::BlockEntity(pos),
                );
            }
            self.game.ensure_stateful_entity_table();
            let at = sunrise_core::position::BlockPos::new(x, y, z);
            let (kept, items, orbs) = self.game.with_level_access(false, |b, level, ticks| {
                let items = level.spill.spawns.len();
                let orbs = level.spill.orbs.len();
                level.drop_tool = Some((key, tool.cloned()));
                level.drop_entity = breaker;
                level.drop_breaker_at = breaker.map(|_| at);
                level.drop_into = collect.then(Vec::new);
                b.drop_resources(level, ticks, at, old);
                (
                    level.drop_into.take().unwrap_or_default(),
                    level.spill.spawns.split_off(items),
                    level.spill.orbs.split_off(orbs),
                )
            });
            collected = kept;
            if !items.is_empty() || !orbs.is_empty() {
                self.game.after_commit(After::BlockDropSpawns(items, orbs));
            }
        }
        collected
    }

    fn queue(&mut self, write: PendingWrite) -> bool {
        self.game.after_commit(After::LevelWrite(Box::new(write)));
        true
    }
}

macro_rules! read_level {
    ($(fn $method:ident(&self $(, $arg:ident: $ty:ty)*) $(-> $result:ty)?;)*) => {$(
        fn $method(&self $(, $arg: $ty)*) $(-> $result)? {
            self.read().$method($($arg),*)
        }
    )*};
}

impl LevelHost for JournalLevelWrite<'_, '_> {
    #[inline]
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        mod_api::BlockStates::of(&*self.game)
    }
    #[inline]
    fn attachment_read_scope(&self) -> mod_api::AttachmentReadScope<'_> {
        self.game.attachment_frame_borrow()
    }
    read_level! {
        fn block_entity(&self, pos: [i32; 3]) -> Option<BlockEntityRef>;
        fn has_item_capability(&self, pos: [i32; 3], side: Option<Direction>) -> Option<bool>;
        fn container_size(&self, at: BlockSide) -> Option<usize>;
        fn lend_container_stack(&self, at: BlockSide, slot: usize, read: &mut dyn for<'a> FnMut(ItemStackView<'a>));
        fn block_entity_part_snapshot(&self, entity: BlockEntityRef, namespace: &str, name: &str) -> Result<AttachmentBatch, PartStateError>;
        fn block_state(&self, pos: [i32; 3]) -> Option<BlockState>;
        fn block_state_id(&self, name: &str) -> Option<BlockState>;
        fn block_state_default(&self, block: BlockKey) -> Option<BlockState>;
        fn state_string(&self, state: BlockState) -> Option<String>;
        fn min_y(&self) -> i32;
        fn height(&self) -> i32;
        fn fluid_state(&self, pos: [i32; 3]) -> Option<FluidState>;
        fn state_with_property(&self, state: BlockState, key: &str, value: &str) -> Option<BlockState>;
        fn brightness(&self, layer: LightLayer, pos: [i32; 3]) -> i32;
        fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32;
        fn is_loaded(&self, pos: [i32; 3]) -> bool;
        fn is_raining_at(&self, pos: [i32; 3]) -> bool;
        fn is_raining(&self) -> Option<bool>;
        fn sky_darken(&self) -> Option<i32>;
        fn enabled_features(&self) -> FeatureFlags;
    }
    #[inline]
    fn state_block(&self, state: BlockState) -> Option<&str> {
        self.game
            .pack
            .assets
            .registry
            .state(state.try_into_engine(&()).ok()?)
            .map(|state| &*state.name)
    }
    #[inline]
    fn tags(&self) -> &dyn Tags {
        self.game
    }
    #[inline]
    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        self.game.block_entity_data(entity)
    }
    #[inline]
    fn signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.game.level_signal(self.level, pos, direction)
    }
    #[inline]
    fn direct_signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.game.level_direct_signal(self.level, pos, direction)
    }
    #[inline]
    fn direct_signal_to(&self, pos: [i32; 3]) -> Option<i32> {
        self.game.level_direct_signal_to(self.level, pos)
    }
    #[inline]
    fn best_neighbor_signal(&self, pos: [i32; 3]) -> Option<i32> {
        self.game.level_best_neighbor_signal(self.level, pos)
    }
    #[inline]
    fn has_neighbor_signal(&self, pos: [i32; 3]) -> Option<bool> {
        Some(self.game.level_has_neighbor_signal(self.level, pos))
    }
    #[inline]
    fn collision_shape(&self, pos: [i32; 3]) -> Option<mod_api::VoxelShape> {
        self.is_loaded(pos).then(|| {
            let [x, y, z] = pos;
            mod_api::__private::voxel_shape_from_core(super::native_state_shapes::collision_shape(
                self.game.collision,
                &self.game.world,
                self.game.world.block(x, y, z),
                pos,
            ))
        })
    }
}

impl LevelRandomHost for JournalLevelWrite<'_, '_> {
    #[inline]
    fn container_signal(&mut self, pos: [i32; 3]) -> Option<i32> {
        self.is_loaded(pos)
            .then(|| self.game.stored_container_signal((pos[0], pos[1], pos[2])))
    }
    #[inline]
    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        self.game
    }
    #[inline]
    fn lend_random(&mut self, draw: RandomDraw<'_>) {
        sunrise_gameplay_common::game::api_values::with_live_level_random(
            &mut self.game.world.random,
            draw,
        );
    }
}

impl LevelTicksHost for JournalLevelWrite<'_, '_> {
    #[inline]
    fn schedule_block_tick(&mut self, [x, y, z]: [i32; 3], block: BlockKey, delay: i32) -> bool {
        let pos = BlockPos::new(x, y, z);
        let Some(block) = sunrise_gameplay_common::game::api_values::behavior_block(
            block,
            self.game.res.behaviours(),
        ) else {
            return false;
        };
        let level = super::Dim::from_id(self.level.dimension_id());
        self.game.block_ticks.blocks.chunk(pos.chunk()).is_some()
            && self.queue(PendingWrite::BlockTick(level, pos, block, delay))
    }
    #[inline]
    fn schedule_fluid_tick(&mut self, [x, y, z]: [i32; 3], fluid: &str, delay: i32) -> bool {
        let pos = BlockPos::new(x, y, z);
        let Some(fluid) = FluidKind::from_name(fluid) else {
            return false;
        };
        let level = super::Dim::from_id(self.level.dimension_id());
        self.game.block_ticks.fluids.chunk(pos.chunk()).is_some()
            && self.queue(PendingWrite::FluidTick(level, pos, fluid, delay))
    }
}

impl LevelWriteHost for JournalLevelWrite<'_, '_> {
    #[inline]
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool {
        state
            .try_into_engine(&())
            .is_ok_and(|state| self.write_block(pos, state, flags.bits()))
    }
    #[inline]
    fn remove_block(&mut self, pos: [i32; 3], moved: bool) -> bool {
        if !self.is_loaded(pos) {
            return false;
        }
        let (replacement, flags) = self.game.level_removal_write(pos, moved);
        self.write_block(pos, replacement, flags)
    }
    #[inline]
    fn set_block_property(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: BlockUpdateFlags,
    ) -> bool {
        let [x, y, z] = pos;
        sunrise_gameplay_common::game::native_hooks::with_property(
            self.game.pack.assets.registry,
            self.game.world.block(x, y, z),
            key,
            value,
        )
        .is_some_and(|state| self.write_block(pos, state, flags.bits()))
    }
    #[inline]
    fn play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if self.game.server.lan.is_none()
            || !sunrise_gameplay_common::game::native_hooks::sound_id(sound)
        {
            return false;
        }
        self.game.emit_level_effect(PendingWrite::Sound {
            sound: NativeSound {
                dimension: self.level.dimension_id(),
                sound: sound.to_owned(),
                inline_range: None,
                at,
                source: source.index() as i32,
                volume,
                pitch,
                seed: 0,
                except: None,
                entity: None,
                skip_hooks: false,
            },
            seed: None,
        });
        true
    }
    #[inline]
    fn send_particles(
        &mut self,
        options: &ParticleOptions,
        state: Option<BlockState>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let (Ok(options), Ok(state)) = (
            options.try_into_engine(GameItems::items(&*self.game)),
            state.try_into_engine(&()),
        ) else {
            return false;
        };
        let registry = self.game.pack.assets.registry;
        if !sunrise_gameplay_common::game::native_hooks::mod_particles_ok(
            &options,
            state,
            |id| registry.state(id).is_some(),
            std::slice::from_ref(&at),
            spread,
            speed,
        ) {
            return false;
        }
        let Some(particle) =
            sunrise_gameplay_common::game::lan_particle_data::server_particle_from_options(
                &options, state,
            )
        else {
            return false;
        };
        self.queue(PendingWrite::Particles(ServerQueuedParticle {
            dimension: self.level.dimension_id(),
            particle,
            at,
            spread,
            speed: [speed; 3],
            count,
            randomization: 0,
            except: None,
        }))
    }
    #[inline]
    fn level_event(&mut self, id: i32, pos: [i32; 3], data: i32) -> bool {
        sunrise_gameplay_common::game::mod_api_host::level_event_data_ok(
            id,
            data,
            self.game.res.facts().states.len(),
        ) && self.queue(PendingWrite::Event {
            level: super::Dim::from_id(self.level.dimension_id()),
            id,
            pos,
            data,
        })
    }
}

impl ServerGame<'_> {
    pub(in crate::game::server_side) fn level_removal_write(
        &mut self,
        pos: [i32; 3],
        moved: bool,
    ) -> (BlockStateId, i32) {
        let [x, y, z] = pos;
        let at = BlockPos::new(x, y, z);
        let state = self.common.world.block(x, y, z);
        let replacement = self.with_level(|b, level, _| {
            level.adjust_remove_block(at, state, moved, b.legacy_block(b.fluid(state)))
        });
        let flags = flags::UPDATE_ALL
            | if moved {
                flags::UPDATE_MOVE_BY_PISTON
            } else {
                0
            };
        (replacement, flags)
    }

    /// Run `f` with `level` in place, temporarily swapping in a parked level and restoring the
    /// active level on exit (including unwind). Never opens an unloaded level from disk; returns
    /// `None` if `level` is neither active nor parked.
    pub(in crate::game) fn with_effect_level<R>(
        &mut self,
        level: super::Dim,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if self.common.dim == level {
            return Some(f(self));
        }
        if self.common.options.execution_role.is_client()
            || !self.server.levels.0.parked.contains_key(&level)
        {
            return None;
        }
        let back = self.common.dim;
        if let Err(e) = self.enter_level(level) {
            log::error!("could not switch to {} for effect replay: {e}", level.id());
            return None;
        }
        struct Restore<'g, 'a> {
            game: &'g mut crate::game::ServerGame<'a>,
            back: super::Dim,
        }
        impl Drop for Restore<'_, '_> {
            fn drop(&mut self) {
                if self.game.dim != self.back
                    && let Err(e) = self.game.enter_level(self.back)
                {
                    log::error!(
                        "could not restore {} after effect replay: {e}",
                        self.back.id()
                    );
                }
            }
        }
        let guard = Restore { game: self, back };
        Some(f(guard.game))
    }

    /// Queue a validated level, entity, or player effect immediately, or defer pure delivery to
    /// root commit when a transaction is open in the active level so rolled-back transactions and
    /// aborted savepoints never publish sounds, particles, or level/global/entity events.
    ///
    /// For transactional sounds, `play_level_sound_at_*` hooks run synchronously here inside the
    /// open transaction so listeners can edit/cancel the sound and have their own writes journaled;
    /// only the resolved packet record is deferred to commit.
    pub(in crate::game::server_side) fn emit_level_effect(&mut self, write: PendingWrite) {
        if self.has_level_transaction() {
            let write = match write {
                PendingWrite::Sound { sound, seed } => match self.resolve_native_sound(sound) {
                    Some(resolved) => PendingWrite::Sound {
                        sound: resolved,
                        seed,
                    },
                    None => return,
                },
                other => other,
            };
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::LevelWrite(Box::new(write)),
            );
            return;
        }
        self.publish_level_effect(write, false);
    }

    pub(in crate::game) fn commit_level_write(&mut self, write: PendingWrite) {
        self.publish_level_effect(write, true);
    }

    /// Shared publication path for immediate calls (`deliver_now = false`) and pure root-commit
    /// replay (`deliver_now = true`). When `deliver_now` is true, each effect replays in its
    /// recorded `Dim` and writes packets directly in journal order without running mod callbacks
    /// or draining unrelated queued effects.
    fn publish_level_effect(&mut self, write: PendingWrite, deliver_now: bool) {
        match write {
            PendingWrite::BlockTick(level, pos, block, delay) => {
                let _ = self.with_effect_level(level, |game| {
                    game.common.block_ticks.schedule_block(
                        game.common.world.game_time,
                        pos,
                        block,
                        delay,
                        TickPriority::Normal,
                    );
                });
            }
            PendingWrite::FluidTick(level, pos, fluid, delay) => {
                let _ = self.with_effect_level(level, |game| {
                    game.common.block_ticks.schedule_fluid(
                        game.common.world.game_time,
                        pos,
                        fluid,
                        delay,
                    );
                });
            }
            PendingWrite::Sound { mut sound, seed } => {
                // The network's sound seed advances only for a published sound, and not for one
                // whose caller gave its seed.
                if let Some(host) = &self.server.lan {
                    sound.seed = seed.unwrap_or_else(|| host.hub.next_sound_seed());
                    if deliver_now {
                        self.send_resolved_native_sound(&sound);
                    } else if self.common.creatures.asking_level_sound {
                        if let Some(resolved) = self.resolve_native_sound(sound) {
                            self.send_resolved_native_sound(&resolved);
                        }
                    } else {
                        self.server.pending_native_sounds.push(sound);
                    }
                }
            }
            PendingWrite::Particles(particles) => {
                if deliver_now {
                    self.server_send_particles(particles);
                } else {
                    self.server.pending_particles.push(particles);
                }
            }
            PendingWrite::Event {
                level,
                id,
                pos,
                data,
            } => {
                let _ = self.with_effect_level(level, |game| {
                    if deliver_now {
                        game.deliver_level_event(id, pos, data);
                    } else {
                        game.server_level_event_at(id, pos, data);
                    }
                });
            }
            PendingWrite::GlobalEvent {
                level,
                id,
                pos,
                data,
            } => {
                let _ = self.with_effect_level(level, |game| {
                    if deliver_now {
                        game.deliver_global_level_event(id, pos, data);
                    } else {
                        game.global_level_event(id, pos, data);
                    }
                });
            }
            PendingWrite::EntityEvent {
                level,
                entity_id,
                event,
                at,
            } => {
                let _ = self.with_effect_level(level, |game| {
                    if deliver_now {
                        game.deliver_entity_event(entity_id, event, at);
                    } else {
                        game.lan_entity_event(entity_id, event, at);
                    }
                });
            }
            PendingWrite::PlayerSound {
                level,
                player,
                sound,
                at,
                source,
                volume,
                pitch,
            } => {
                let _ = self.with_effect_level(level, |game| {
                    let Some(target) = game.native_player_sound_target(player) else {
                        return;
                    };
                    let seed = game.world.random.next_long();
                    game.deliver_player_sound(target, &sound, at, volume, pitch, source, seed);
                });
            }
            PendingWrite::PlayerStopSound {
                level,
                player,
                sound,
                source,
            } => {
                let _ = self.with_effect_level(level, |game| {
                    let Some(target) = game.native_player_sound_target(player) else {
                        return;
                    };
                    game.deliver_player_stop_sound(target, sound.as_deref(), source);
                });
            }
        }
    }

    /// `LiquidBlockContainer.placeLiquid`: what the part writes stays in the ambient transaction
    /// whatever it answers. A panicking part's scope aborts with it.
    pub(in crate::game::server_side) fn journalled_place_liquid(
        &mut self,
        level: LevelRef,
        write: &FluidWrite,
    ) -> Option<bool> {
        if !sunrise_gameplay_common::game::api_values::block_state::block_listens(
            mod_api::HookId::PlaceLiquid,
            write.from,
        ) || !self.native_level_write_allowed(level, [write.pos.x, write.pos.y, write.pos.z])
        {
            return None;
        }
        let state = write.from.try_into_api().ok()?;
        let fluid = sunrise_gameplay_common::game::native_world_read_common::api_fluid_state(
            write.source_fluid_state(),
        )?;
        let mut answer = None;
        mod_api::dispatch_place_liquid_with(write.pos, state, fluid, &mut answer, |part, event| {
            // For a player bucket this can be the root scope; the fluid
            // utility already has an outer transaction. Unwinding aborts this
            // scope before the owner dispatcher catches a Part panic.
            self.storage_scope(|game, _| {
                let reply = JournalLevelWrite::new(game, level)
                    .and_then(|mut host| part.place_liquid(&mut host, event));
                (reply, true)
            })
        });
        answer
    }
}

use sunrise_gameplay_common::game::tables::GameItems;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
