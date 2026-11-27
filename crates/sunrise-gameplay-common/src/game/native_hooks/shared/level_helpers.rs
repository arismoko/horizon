use super::*;
use sunrise_level::containers::ModBlockEntity;

/// Server-level helpers for native mods.
impl GameLevel<'_> {
    /// Play a sound event for nearby players in this server level. The return
    /// value means queued, not heard. The server sends registered sounds by ID and other valid
    /// keys by name (`Holder.direct`). The receiving client resolves the sound resources.
    pub(in crate::game) fn native_play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if !self.native_authoritative || !sound_id(sound) {
            return false;
        }
        // `Level.playSound` draws `soundSeedGenerator.nextLong` at the call;
        // without a host nothing can be sent.
        let Some(seed) = self.sound_seeds.map(|hub| hub.next_sound_seed()) else {
            return false;
        };
        self.native_sounds
            .push(crate::game::creatures::NativeSound {
                dimension: dimension_id(self.dimension),
                sound: sound.to_owned(),
                inline_range: None,
                at,
                source: source.index() as i32,
                volume,
                pitch,
                seed,
                except: None,
                entity: None,
                skip_hooks: false,
            });
        true
    }

    /// Queue particles in this authoritative level. A zero count sends one
    /// directional particle. The return value means accepted, not delivered.
    /// Block particle types require their block state; item, trail and
    /// vibration particles are not supported by the current network dispatch.
    /// `ParticleOptions::named("mymod:spark")` sends a mod's own type
    ///: Sunrise clients draw it from the sprite
    /// set in the mod's `assets/<namespace>/particles/<path>.json`, a client
    /// without that file draws nothing, and vanilla clients aren't sent it.
    pub(in crate::game) fn native_send_particles(
        &mut self,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let state_ok = |id| self.registry.state(id).is_some();
        if !self.native_authoritative
            || !mod_particles_ok(
                options,
                state,
                state_ok,
                std::slice::from_ref(&at),
                spread,
                speed,
            )
        {
            return false;
        }
        let Some(particle) =
            crate::game::lan_particle_data::server_particle_from_options(options, state)
        else {
            return false;
        };
        self.native_particles
            .push(sunrise_sim::lan::server_effects::ServerQueuedParticle {
                dimension: dimension_id(self.dimension),
                particle,
                at,
                spread,
                speed: [speed; 3],
                count,
                randomization: 0,
                except: None,
            });
        true
    }
    /// `EntityBlock.newBlockEntity` of block `name`: a declared mod type
    /// starts with the data `native_block_entity` gives, a mod's sign, else
    /// Sunrise's own. The one place both whole-block write paths make it.
    pub(in crate::game) fn new_block_entity(&self, name: &str) -> Option<BlockEntity> {
        let mut nbt = None;
        if sunrise_game_data::block_entity_types::type_for(name).is_some() {
            self.native_block_entity(name, self.registry.default_state(name)?, &mut nbt);
        }
        block_entity_for_block(self.registry, self.facts, name, nbt.unwrap_or_default())
    }
    /// Creates a declared state-aware block entity from its actual state. No answer
    /// means no entity; the registered declaration owns its type identity.
    pub fn new_block_entity_for_state(&self, state: BlockStateId) -> Option<BlockEntity> {
        let placed = self.registry.state(state)?;
        let name = &placed.name;
        if self.world.block_entities.stateful_entity(state.0) {
            let mut nbt = None;
            self.native_block_entity(name, state, &mut nbt);
            return block_entity_for_block(self.registry, self.facts, name, nbt?);
        }
        let mut entity = self.new_block_entity(name)?;
        // `TestBlockEntity` takes its mode from the state at construction,
        // before any block entity data supplied by the item is loaded.
        if let BlockEntity::TestBlock(test) = &mut entity
            && let Some(mode) = placed
                .properties
                .get("mode")
                .and_then(|mode| sunrise_level::test_block::TestBlockMode::of_serialized_name(mode))
        {
            test.mode = mode;
        }
        Some(entity)
    }
    /// Replaces the block at `pos` with `state` through `Level.setBlock`, as
    /// `Game::native_set_block` does for the Host, from a hook that gets the
    /// level's `ticks`: the old block's block entity is removed and the new
    /// block's is created in the write (`LevelChunk.setBlockState`), and the
    /// write's level events stay queued for the caller, in order; a mod block
    /// entity starts with the data `native_block_entity` gives. True when the block changed; false off an
    /// authoritative server, for a position that is not loaded or outside the
    /// build height, an invalid state, or an unchanged state. Never loads a
    /// chunk.
    pub(in crate::game) fn native_set_block(
        &mut self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        if !self.native_authoritative
            || !self.world.is_loaded_at(pos[0], pos[1], pos[2])
            || self.registry.state(state).is_none()
        {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        let b = self.behaviours;
        let changed = b.set_block(self, ticks, at, state, flags);
        self.finish_native_block(pos, previous, state, changed);
        changed
    }

    pub fn finish_native_block(
        &mut self,
        pos: [i32; 3],
        previous: BlockStateId,
        state: BlockStateId,
        changed: bool,
    ) {
        let landed = self.world.block(pos[0], pos[1], pos[2]);
        if !self.world.block_entities.stateful_entity(landed.0)
            && block_entity_to_create(self.facts, changed, previous, state, landed).is_some()
            && let Some(entity) = self.new_block_entity_for_state(landed)
        {
            put_block_entity(self.world, pos, entity);
        }
    }
    /// `Level.removeBlock`: replace a block with its fluid's legacy block.
    pub(in crate::game) fn remove_block_write(
        &mut self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        moved: bool,
    ) -> bool {
        if !self.native_authoritative || !self.world.is_loaded_at(pos[0], pos[1], pos[2]) {
            return false;
        }
        self.behaviours
            .remove_block(self, ticks, BlockPos::new(pos[0], pos[1], pos[2]), moved)
    }
    /// Schedules a tick of `block` (a block name) at `pos` in `delay` game
    /// ticks, as a falling block's shape update schedules its fall
    /// (`ScheduledTickAccess.scheduleTick(pos, block, delay)`, normal
    /// priority); false for an unknown block or a position that isn't
    /// loaded. A tick of that block already queued there is kept and counts
    /// as scheduled. `ticks` is the hook's.
    pub(in crate::game) fn native_schedule_block_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        block: BlockKey,
        delay: i32,
    ) -> bool {
        let Some(block) = crate::game::api_values::behavior_block(block, self.behaviours) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let time = self.game_time();
        ticks.schedule_block(time, at, block, delay, TickPriority::Normal)
            != ScheduleResult::MissingChunk
    }
    /// Schedules a tick of `fluid` (`minecraft:water`, `minecraft:lava`,
    /// `minecraft:flowing_water`...) at `pos` in `delay` game ticks, as a
    /// waterlogged block's shape update schedules its water in 5
    /// (`ScheduledTickAccess.scheduleTick(pos, fluid, delay)`); false for
    /// an unknown fluid or a position that isn't loaded. A tick of that
    /// fluid already queued there is kept and counts as scheduled. `ticks` is
    /// the hook's.
    pub(in crate::game) fn native_schedule_fluid_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        fluid: &str,
        delay: i32,
    ) -> bool {
        let Some(kind) = FluidKind::from_name(fluid) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let time = self.game_time();
        ticks.schedule_fluid(time, at, kind, delay) != ScheduleResult::MissingChunk
    }
    /// The state `state` with its property `key` set to `value`, as a mod block
    /// builds the state it is placed in: `native_state_with_property(cable,
    /// "facing", "east")` (`BlockState.setValue`). A block's own declared
    /// properties (`state_properties` in its block file) are set the same way.
    /// `state` itself when the property already has that value; None when the
    /// block has no such property, or the property no such value.
    pub(in crate::game) fn native_state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        with_property(self.registry, state, key, value)
    }
    /// The block `state` is a state of (`BlockState.getBlock()`): its name,
    /// `mymod:cable`. `native_state_for_placement` is asked for every block
    /// placed and given the block's default state: compare this with your
    /// block to answer only for it. None for an unknown state.
    pub(in crate::game) fn native_state_block(&self, state: BlockStateId) -> Option<&str> {
        self.registry.state(state).map(|s| &*s.name)
    }
}

/// `EntityBlock.newBlockEntity` of block `name`: a declared mod type starting
/// with `initial` data, a mod's sign, else Sunrise's own. Shared by the
/// Host's and a level hook's whole-block writes.
pub fn block_entity_for_block(
    registry: &BlockRegistry,
    facts: &sunrise_items::blocks::BlockFacts,
    name: &str,
    initial: Compound,
) -> Option<BlockEntity> {
    if let Some(kind) = sunrise_game_data::block_entity_types::type_for(name) {
        let slots = mod_api::__private::parts_installation::block_container(name)
            .map_or(0, |container| container.slots);
        return Some(BlockEntity::Mod(Box::new(ModBlockEntity::new(
            kind.to_owned(),
            ModBlockEntityFields {
                nbt: initial,
                items: vec![None; slots],
                data: Vec::new(),
            },
        ))));
    }
    // Undeclared content blocks retain their template's vanilla behaviour.
    // `SignBlock.newBlockEntity`: a mod's sign is one by its template's class.
    if !name.starts_with("minecraft:")
        && let Some(state) = registry.default_state(name)
        && let Some(form) = facts.get(state).sign_form()
    {
        let text = facts.sign_text(name).map_or_else(Default::default, |d| {
            sunrise_level::signs_data::SignText {
                color: d.color,
                glowing: d.glowing,
                ..Default::default()
            }
        });
        return Some(BlockEntity::Sign(sunrise_level::signs_data::SignData {
            hanging: form.hanging,
            front: text.clone(),
            back: text,
            ..Default::default()
        }));
    }
    BlockEntity::for_block(name)
}

/// After a `Level.setBlock` of `state` over `previous`: the block whose block
/// entity the write creates, when the write changed the block's name and the
/// new block landed.
fn block_entity_to_create(
    facts: &sunrise_items::blocks::BlockFacts,
    changed: bool,
    previous: BlockStateId,
    state: BlockStateId,
    landed: BlockStateId,
) -> Option<String> {
    let new = &facts.get(state).name;
    (changed && facts.get(previous).name != *new && facts.get(landed).name == *new)
        .then(|| new.to_string())
}

/// Stores a new block entity at `pos`, keeping an existing one of the same kind.
fn put_block_entity(world: &mut sunrise_level::world::World, pos: [i32; 3], entity: BlockEntity) {
    let key = (pos[0], pos[1], pos[2]);
    if let Some(existing) = world.block_entities.get(&key) {
        if std::mem::discriminant(existing) == std::mem::discriminant(&entity) {
            return;
        }
        // During a spill join the old row can still be in place. Retire its saved tag
        // before installing the replacement; the outer removal skips the new generation.
        world.forget_block_entity_source(key);
    }
    world.block_entities.insert(key, entity);
}
