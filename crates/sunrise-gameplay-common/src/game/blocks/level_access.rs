//! The world behaviour adapter, kept beside the block operations it serves.
use super::*;
use sunrise_level::sculk::SculkKind;

/// `SculkSensorBlock.getAnalogOutputSignal`: the last vibration's frequency
/// while the sensor is active.
pub(in crate::game) fn sensor_signal(
    world: &sunrise_level::world::World,
    behaviours: &sunrise_world::level::Behaviours,
    state: BlockStateId,
    pos: BlockPos,
) -> Option<i32> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
        Some(BlockEntity::Sculk(e))
            if matches!(e.kind, SculkKind::Sensor | SculkKind::Calibrated) =>
        {
            Some(if behaviours.sculk_sensor_active(state) {
                e.last_frequency
            } else {
                0
            })
        }
        _ => None,
    }
}

impl LevelAccess for GameLevel<'_> {
    #[inline]
    fn infiniburn(&self) -> Option<&sunrise_world::level::data::BlockSet> {
        self.world.infiniburn.as_deref()
    }

    #[inline]
    fn golem_heads(&self) -> Option<&sunrise_world::level::data::BlockSet> {
        self.world.golem_heads.as_deref()
    }
    #[inline]
    fn cake_can_eat(&self, vanilla: bool) -> bool {
        if self.cake_probe {
            self.cake_requested.set(true);
            false
        } else {
            self.cake_answer.unwrap_or(vanilla)
        }
    }

    #[inline]
    fn within_world_border(&self, pos: BlockPos) -> bool {
        self.border
            .is_within_bounds(f64::from(pos.x), f64::from(pos.z), 0.0)
    }
    #[inline]
    fn redstone_experiments(&self) -> bool {
        self.redstone_experiments
    }
    #[inline]
    fn override_place_sound(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        placer: Option<u32>,
    ) -> Option<sunrise_world::level::data::BlockSound> {
        let cause = mod_api::BlockSoundCause::Place;
        let level = sunrise_sim::motion::FrictionLevel {
            world: &*self.world,
            registry: self.registry,
            context: self.loop_context,
            side: &self.side,
            coefficient: &|_| None,
        };
        let sound = sunrise_sim::block_sounds::native_sound_type(
            level,
            pos,
            state,
            placer.map(|id| (id, "minecraft:player")),
            cause,
            &self.facts.get(state).sounds,
        )?;
        Some(sunrise_world::level::data::BlockSound::from_type(
            &sound, cause,
        ))
    }
    #[inline]
    fn allow_hopper_pickup_item(&self, pos: BlockPos, item: &str, count: u32) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_hopper_pickup_item(self.dimension, pos, item, count, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn has_native_block_entity(&self, state: BlockStateId) -> bool {
        self.world.block_entities.stateful_entity(state.0)
    }
    #[inline]
    fn ensure_block_entity(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        if !self.world.block_entities.stateful_entity(state.0)
            || self.world.block_entities.get(&key).is_some()
        {
            return;
        }
        if let Some(entity) = self.new_block_entity_for_state(state) {
            self.world.block_entities.insert(key, entity);
        }
    }
    #[inline]
    fn observe_block_changed(
        &self,
        pos: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
        flags: i32,
    ) {
        if self.native_authoritative {
            self.native_block_changed(
                self.dimension,
                pos,
                old,
                new,
                mod_api::BlockUpdateFlags::from_bits_retain(flags),
            );
        }
    }
    #[inline]
    fn observes_block_state_change(&self, old: BlockStateId, new: BlockStateId) -> bool {
        self.block_state_change_heard(old, new)
    }
    #[inline]
    fn observe_block_state_change(&self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        self.block_state_change(pos, old, new, false);
    }
    #[inline]
    fn neighbor_notify(&self, pos: BlockPos, skipped: Option<Direction>) -> bool {
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::NeighborNotify)
        {
            return false;
        }
        let mut cancel = false;
        let state = self.block_state(pos);
        self.native_neighbor_notify(self.dimension, pos, state, skipped, &mut cancel);
        cancel
    }
    #[inline]
    fn observe_neighbor_changed(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        changed: sunrise_world::level::BlockId,
        moved: bool,
    ) {
        if self.native_authoritative
            && api_values::block_state::block_listens(mod_api::HookId::NeighborChanged, state)
        {
            use api_values::TryIntoApi;
            let Ok(changed) = self.behaviours.table.default_state(changed).try_into_api() else {
                return;
            };
            self.native_neighbor_changed(ticks, self.dimension, pos, state, changed.block(), moved);
        }
    }

    fn observe_on_place(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        old: BlockStateId,
        moved: bool,
    ) {
        if self.native_authoritative
            && api_values::block_state::block_listens(mod_api::HookId::OnPlace, state)
        {
            self.native_on_place(ticks, self.dimension, pos, state, old, moved);
        }
    }
    #[inline]
    fn affect_neighbors_after_removal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        moved: bool,
        updates: &mut Vec<sunrise_core::neighbour_update::NeighbourUpdate>,
    ) {
        if self.native_authoritative {
            self.native_affect_neighbors_after_removal(self.dimension, pos, state, moved, updates);
        }
    }
    #[inline]
    fn allow_spread(
        &self,
        kind: sunrise_world::level::Spread,
        from: BlockPos,
        to: BlockPos,
    ) -> bool {
        let mut canceled = false;
        if self.native_authoritative {
            let kind = match kind {
                sunrise_world::level::Spread::Fluid => "fluid",
                sunrise_world::level::Spread::Fire => "fire",
            };
            self.native_block_spread(self.dimension, kind, from, to, &mut canceled);
        }
        !canceled
    }
    #[inline]
    fn has_liquid_container_part(&self, state: BlockStateId) -> bool {
        super::api_values::block_state::block_listens(mod_api::HookId::CanPlaceLiquid, state)
    }
    #[inline]
    fn can_place_liquid(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        fluid: sunrise_world::fluid::FluidKind,
        user: sunrise_world::level::LiquidUser,
    ) -> Option<bool> {
        if !self.has_liquid_container_part(state) {
            return None;
        }
        let fluid = crate::game::native_world_read_common::api_fluid_state(
            sunrise_world::fluid::FluidState::new(fluid, 8, false),
        )?
        .fluid;
        let user = self
            .api_level
            .zip(user.entity_id)
            .map(|(level, id)| mod_api::LivingRef::new(mod_api::EntityRef::new(level.epoch(), id)));
        let mut answer = None;
        self.native_can_place_liquid(pos, state, fluid, user, &mut answer);
        answer
    }
    #[inline]
    fn has_place_liquid_part(&self, state: BlockStateId) -> bool {
        self.native_authoritative
            && super::api_values::block_state::block_listens(mod_api::HookId::PlaceLiquid, state)
    }
    #[inline]
    fn place_liquid(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        fluid: sunrise_world::fluid::FluidState,
    ) -> Option<bool> {
        if !self.has_place_liquid_part(state) || !self.world.is_loaded_at(pos.x, pos.y, pos.z) {
            return None;
        }
        if self.liquid_transaction_open {
            log::warn!(
                "Sunrise: refused direct place_liquid during an open transaction at {pos:?}"
            );
            return None;
        }
        let fluid = crate::game::native_world_read_common::api_fluid_state(fluid)?;
        let mut answer = None;
        self.native_place_liquid(ticks, pos, state, fluid, &mut answer);
        answer
    }

    #[inline]
    fn fluid_interaction_test(
        &self,
        handler: u32,
        pos: BlockPos,
        relative: BlockPos,
        fluid: sunrise_world::fluid::FluidState,
    ) -> bool {
        let Some((owner, handler)) = super::fluid_interactions::native_fluid_interaction(handler)
        else {
            return false;
        };
        let Some(state) = crate::game::native_world_read_common::api_fluid_state(fluid) else {
            return false;
        };
        if !self.native_authoritative {
            return false;
        }
        let at = mod_api::FluidInteractionAt {
            pos,
            relative,
            state,
        };
        let level: &dyn mod_api::LevelHost = self;
        owner
            .call("fluid interaction", "test", || handler.test(level, at))
            .unwrap_or_else(|error| {
                log::warn!("Sunrise: {error}; its fluid interaction does not apply");
                false
            })
    }

    #[inline]
    fn fluid_interaction_interact(
        &mut self,
        ticks: &mut Ticks,
        handler: u32,
        pos: BlockPos,
        relative: BlockPos,
        fluid: sunrise_world::fluid::FluidState,
    ) {
        let Some((owner, handler)) = super::fluid_interactions::native_fluid_interaction(handler)
        else {
            return;
        };
        let Some(state) = crate::game::native_world_read_common::api_fluid_state(fluid) else {
            return;
        };
        if self.liquid_transaction_open {
            log::warn!(
                "Sunrise: refused a direct fluid interaction during an open transaction at {pos:?}"
            );
            return;
        }
        let at = mod_api::FluidInteractionAt {
            pos,
            relative,
            state,
        };
        let mut host = super::mod_api_host::LevelTicks::new(&mut *self, ticks);
        if let Err(error) = owner.call("fluid interaction", "interact", || {
            handler.interact(&mut host, at)
        }) {
            log::warn!("Sunrise: {error}; its fluid interaction stopped");
        }
    }
    #[inline]
    fn fluid_place_block(
        &self,
        pos: BlockPos,
        liquid_pos: BlockPos,
        state: BlockStateId,
    ) -> Option<BlockStateId> {
        let mut new_state = state;
        let mut cancel = false;
        if self.native_authoritative {
            self.native_fluid_place_block(
                self.dimension,
                pos,
                liquid_pos,
                state,
                &mut new_state,
                &mut cancel,
            );
        }
        (!cancel).then_some(new_state)
    }
    #[inline]
    fn allow_fluid_tick(&self, pos: BlockPos, fluid: &'static str) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_fluid_tick(self.dimension, pos, fluid, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn allow_fluid_level_change(
        &self,
        pos: BlockPos,
        fluid: &'static str,
        from: i32,
        to: i32,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_fluid_level_change(self.dimension, pos, fluid, from, to, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn create_fluid_source(&self, pos: BlockPos, state: BlockStateId, vanilla: bool) -> bool {
        let mut can_convert = vanilla;
        if self.native_authoritative {
            self.native_create_fluid_source(self.dimension, pos, state, &mut can_convert);
        }
        can_convert
    }
    #[inline]
    fn mod_fluid_rules(
        &self,
        pos: BlockPos,
        fluid: sunrise_world::fluid::ModFluid,
        registered: sunrise_world::fluid::ModFluidRules,
    ) -> sunrise_world::fluid::ModFluidRules {
        if !self.native_authoritative {
            return registered;
        }
        Self::answered_fluid_rules(self.dimension, pos, fluid, registered, |call| {
            self.native_fluid_rules(call)
        })
    }

    fn fluid_passage(
        &self,
        fluid: &'static str,
        from: BlockPos,
        from_state: BlockStateId,
        direction: Direction,
        to: BlockPos,
        to_state: BlockStateId,
    ) -> bool {
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::FluidPassage)
        {
            return true;
        }
        let mut passes = true;
        self.native_fluid_passage(
            self.dimension,
            fluid,
            from,
            from_state,
            direction,
            to,
            to_state,
            &mut passes,
        );
        passes
    }
    #[inline]
    fn allow_block_ignite(&self, pos: BlockPos, cause: &'static str) -> bool {
        self.block_ignite_allowed(pos, cause)
    }
    #[inline]
    fn block_grow_feature<'a>(
        &mut self,
        pos: BlockPos,
        feature: Option<&'a str>,
    ) -> Option<std::borrow::Cow<'a, str>> {
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::BlockGrowFeature)
        {
            return feature.map(std::borrow::Cow::Borrowed);
        }
        let mut feature = feature.map(str::to_owned);
        let mut cancel = false;
        self.native_block_grow_feature(self.dimension, pos, &mut feature, &mut cancel);
        feature.filter(|_| !cancel).map(std::borrow::Cow::Owned)
    }
    #[inline]
    fn crop_grow_pre(&mut self, pos: BlockPos, state: BlockStateId, vanilla: bool) -> bool {
        if !self.native_authoritative || !mod_api::__private::listened(mod_api::HookId::CropGrowPre)
        {
            return vanilla;
        }
        let mut result = mod_api::TriState::Default;
        self.native_crop_grow_pre(self.dimension, pos, state, &mut result);
        result.or(vanilla)
    }
    #[inline]
    fn crop_grow_post(&mut self, pos: BlockPos, original: BlockStateId) {
        if self.native_authoritative && mod_api::__private::listened(mod_api::HookId::CropGrowPost)
        {
            let state = self.block_state(pos);
            self.native_crop_grow_post(self.dimension, pos, original, state);
        }
    }
    #[inline]
    fn allow_block_burn(&self, pos: BlockPos) -> bool {
        self.block_burn_allowed(pos)
    }
    #[inline]
    fn allow_leaves_decay(&self, pos: BlockPos) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_leaves_decay(self.dimension, pos, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn has_water_ticket(&self, pos: BlockPos) -> bool {
        self.world.water_tickets.contains([pos.x, pos.y, pos.z])
    }
    #[inline]
    fn allow_moisture_change(&self, pos: BlockPos, from: i32, to: i32) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_moisture_change(self.dimension, pos, from, to, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn observe_sponge_absorb(&self, pos: BlockPos) {
        if self.native_authoritative {
            self.native_sponge_absorb(self.dimension, pos);
        }
    }
    #[inline]
    fn allow_block_fade(&self, pos: BlockPos, from: BlockStateId, to: BlockStateId) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_fade(self.dimension, pos, from, to, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn allow_block_form(&self, pos: BlockPos, from: BlockStateId, to: BlockStateId) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_form(self.dimension, pos, from, to, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn note_block_change(
        &self,
        pos: BlockPos,
        instrument: u8,
        old: i32,
        mut new: i32,
    ) -> Option<i32> {
        let mut cancel = false;
        if self.native_authoritative {
            let context = mod_api::NoteBlockChange {
                dimension: self.dimension,
                pos,
                instrument: mod_api::NoteInstrument::ALL[usize::from(instrument)],
                old_note: old,
            };
            self.native_note_block_change(&context, &mut new, &mut cancel);
        }
        (!cancel).then_some(new)
    }
    #[inline]
    fn note_block_play(
        &self,
        pos: BlockPos,
        by_player: bool,
        instrument: &mut u8,
        note: &mut i32,
    ) -> bool {
        use mod_api::NoteInstrument;
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::NoteBlockPlay)
        {
            return true;
        }
        let mut played_instrument = NoteInstrument::ALL[usize::from(*instrument)];
        let mut played = *note;
        let mut cancel = false;
        let (player, player_entity_id) = if by_player {
            (self.native_player, self.native_player_entity)
        } else {
            (None, None)
        };
        self.native_note_block_play(mod_api::__private::NoteBlockPlayCall {
            dimension: self.dimension,
            player,
            player_entity_id,
            pos,
            instrument: &mut played_instrument,
            note: &mut played,
            cancel: &mut cancel,
        });
        *instrument = played_instrument as u8;
        // A note out of 0 to 24 keeps the block's own.
        if (0..=24).contains(&played) {
            *note = played;
        }
        !cancel
    }
    #[inline]
    fn allow_portal_spawn(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        shape: &sunrise_world::level::PortalShape,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative && mod_api::__private::listened(mod_api::HookId::PortalSpawn) {
            let b = shape.bottom_left;
            let shape = mod_api::PortalShape::from(mod_api::__private::built::PortalShape {
                x_axis: shape.x_axis,
                bottom_left: [b.x, b.y, b.z],
                width: shape.width,
                height: shape.height,
            });
            self.native_portal_spawn(self.dimension, pos, state, shape, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn allow_bell_ring(&self, pos: BlockPos) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_bell_ring(self.dimension, self.native_player, pos, &mut cancel);
        }
        !cancel
    }
    #[inline]
    fn allow_cauldron_level_change(
        &self,
        pos: BlockPos,
        from: i32,
        to: i32,
        reason: &'static str,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_cauldron_level_changed(mod_api::__private::CauldronLevelChangedCall {
                dimension: self.dimension,
                player: self.native_player,
                pos,
                from_level: from,
                to_level: to,
                reason,
                cancel: &mut cancel,
            });
        }
        !cancel
    }
    #[inline]
    fn allow_piston_move(
        &self,
        piston: BlockPos,
        direction: sunrise_core::geometry::Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) -> bool {
        self.piston_move_allowed(piston, direction, extending, push, destroy)
    }
    #[inline]
    fn on_destroyed_by_push_reaction(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        fluid: FluidState,
    ) {
        self.destroyed_by_push_reaction(ticks, pos, state, direction, fluid);
    }
    #[inline]
    fn observe_piston_moved(
        &self,
        piston: BlockPos,
        direction: sunrise_core::geometry::Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) {
        self.piston_moved(piston, direction, extending, push, destroy);
    }
    #[inline]
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut source = vanilla;
        if self.native_authoritative {
            self.native_is_signal_source(state, &mut source);
        }
        source
    }
    #[inline]
    fn adjust_is_slime_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        self.part_is_slime_block(state, vanilla)
    }
    #[inline]
    fn adjust_is_sticky_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        self.part_is_sticky_block(state, vanilla)
    }
    #[inline]
    fn adjust_can_stick_to(&self, state: BlockStateId, other: BlockStateId, vanilla: bool) -> bool {
        self.part_can_stick_to(state, other, vanilla)
    }
    #[inline]
    fn has_weak_power_hook(&self) -> bool {
        self.native_authoritative
            && mod_api::__private::has_scoped_hook(mod_api::HookId::ShouldCheckWeakPower)
    }
    #[inline]
    fn has_signal_hook(&self) -> bool {
        self.native_authoritative
            && (mod_api::__private::has_scoped_hook(mod_api::HookId::BlockSignal)
                || mod_api::__private::has_scoped_hook(mod_api::HookId::IsSignalSource))
    }
    #[inline]
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool {
        let mut check = vanilla;
        self.native_should_check_weak_power(self.dimension, pos, state, direction, &mut check);
        check
    }
    #[inline]
    fn has_rail_hook(&self) -> bool {
        self.rail_hooked()
    }
    #[inline]
    fn adjust_rail_direction(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        vanilla: sunrise_world::level::RailShape,
    ) -> sunrise_world::level::RailShape {
        self.part_rail_direction(pos, state, vanilla)
    }
    #[inline]
    fn adjust_rail_can_make_slopes(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        vanilla: bool,
    ) -> bool {
        self.part_rail_can_make_slopes(pos, state, vanilla)
    }
    #[inline]
    fn adjust_rail_flexible(&self, pos: BlockPos, state: BlockStateId, vanilla: bool) -> bool {
        self.part_rail_flexible(pos, state, vanilla)
    }
    #[inline]
    fn adjust_rail_valid_shape(
        &self,
        state: BlockStateId,
        shape: sunrise_world::level::RailShape,
        vanilla: bool,
    ) -> bool {
        self.part_rail_valid_shape(state, shape, vanilla)
    }
    #[inline]
    fn has_redstone_wire_connection_hook(&self) -> bool {
        self.native_authoritative
            && mod_api::__private::has_scoped_hook(mod_api::HookId::ShouldRedstoneWireConnectTo)
    }
    #[inline]
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool {
        let mut connected = vanilla;
        self.native_should_redstone_wire_connect_to(
            self.dimension,
            pos,
            state,
            direction,
            &mut connected,
        );
        connected
    }
    #[inline]
    fn adjust_remove_block(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        moved_by_piston: bool,
        vanilla: BlockStateId,
    ) -> BlockStateId {
        self.removal_replacement(pos, state, moved_by_piston, vanilla)
    }
    #[inline]
    fn has_connection_hook(&self) -> bool {
        self.native_authoritative
            && (mod_api::__private::has_scoped_hook(mod_api::HookId::ConnectsTo)
                || mod_api::__private::has_scoped_hook(mod_api::HookId::CanBeConnectedTo))
    }
    #[inline]
    fn adjust_can_be_connected_to(
        &self,
        kind: sunrise_core::connection_kind::ConnectionKind,
        pos: BlockPos,
        state: BlockStateId,
        connector: BlockStateId,
        direction: Direction,
        face_solid: bool,
        vanilla: bool,
    ) -> bool {
        self.can_be_connected_to(kind, pos, state, connector, direction, face_solid, vanilla)
    }
    #[inline]
    fn adjust_connects_to(
        &self,
        kind: sunrise_core::connection_kind::ConnectionKind,
        pos: BlockPos,
        state: BlockStateId,
        neighbour: BlockStateId,
        direction: Direction,
        face_solid: bool,
        vanilla: bool,
    ) -> bool {
        let mut connected = vanilla;
        self.native_connects_to(native_hooks::ConnectsToCall {
            dimension: self.dimension,
            pos,
            state,
            kind,
            neighbour,
            direction,
            face_solid,
            connected: &mut connected,
        });
        connected
    }
    #[inline]
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        native_level_signals::adjust_signal(direction, vanilla, |face, signal| {
            if self.native_authoritative {
                self.native_block_signal(self.dimension, pos, state, face, direct, signal);
            }
        })
    }
    #[inline]
    fn adjust_has_analog_output(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut output = vanilla;
        if self.native_authoritative {
            self.native_has_analog_output(state, &mut output);
        }
        output
    }
    #[inline]
    fn adjust_analog_output(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: i32,
    ) -> i32 {
        let mut output = vanilla;
        if self.native_authoritative {
            self.native_analog_output(self.dimension, pos, state, direction, &mut output);
        }
        output.clamp(0, 15)
    }
    #[inline]
    fn before_random_tick(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: &mut BlockStateId,
        handled: &mut bool,
    ) {
        if self.native_authoritative {
            // Any block the hook writes moves the world's revision on.
            let revision = self.world.revision;
            self.native_random_tick(ticks, self.dimension, pos, *state, handled);
            if !*handled && self.world.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    #[inline]
    fn has_held_block_ticks(&self) -> bool {
        self.native_authoritative && mod_api::__private::any_mod_disabled()
    }
    #[inline]
    fn hold_block_tick(&self, state: BlockStateId) -> bool {
        self.native_authoritative
            && api_values::block_state::block_listens(mod_api::HookId::TickBlock, state)
            && state.try_into_api().is_ok_and(|state| {
                mod_api::__private::block_hook_inert(mod_api::HookId::TickBlock, state)
            })
    }
    #[inline]
    fn before_tick_block(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: &mut BlockStateId,
        handled: &mut bool,
    ) {
        if self.native_authoritative {
            // Any block the hook writes moves the world's revision on.
            let revision = self.world.revision;
            self.native_tick_block(ticks, self.dimension, pos, *state, handled);
            if !*handled && self.world.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    #[inline]
    fn before_update_shape(
        &self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
        new: &mut Option<BlockStateId>,
    ) {
        if self.native_authoritative {
            self.native_update_shape(
                ticks,
                native_hooks::UpdateShapeCall {
                    dimension: self.dimension,
                    pos,
                    state,
                    direction,
                    neighbour_pos,
                    neighbour,
                    new,
                },
            );
        }
    }
    #[allow(clippy::option_option)]
    #[inline]
    fn before_state_for_placement(
        &self,
        ticks: &Ticks,
        preview: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        context: &PlacementContext,
        fluid: FluidState,
        answer: &mut Option<Option<BlockStateId>>,
    ) {
        // Asked on both sides: a client's placement prediction asks it too.
        if let Some(fluid) = crate::game::native_world_read_common::api_fluid_state(fluid) {
            let level = self.view(ticks).previewing(preview);
            super::native_hooks::LevelView::new(&level, self.world).native_state_for_placement(
                self.dimension,
                state,
                context,
                fluid,
                answer,
            );
        }
    }
    #[inline]
    fn must_survive(&self, item: &str) -> Option<bool> {
        let mut answer = None;
        self.native_must_survive(item, &mut answer);
        answer
    }
    #[inline]
    fn fence_gate_in_wall(&self, state: BlockStateId) -> Option<bool> {
        let mut answer = None;
        self.native_fence_gate_in_wall(state, &mut answer);
        answer
    }
    #[inline]
    fn before_can_survive(
        &self,
        ticks: &Ticks,
        preview: Option<&sunrise_world::level::PreviewBlocks>,
        pos: BlockPos,
        state: BlockStateId,
        survives: &mut Option<bool>,
    ) -> bool {
        let mut inert = false;
        let level = self.view(ticks).previewing(preview);
        super::native_hooks::LevelView::new(&level, self.world).native_can_survive(
            self.dimension,
            pos,
            state,
            survives,
            &mut inert,
        );
        inert
    }
    #[inline]
    fn before_can_sustain_plant(
        &self,
        ticks: &Ticks,
        preview: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        sustains: &mut Option<bool>,
    ) -> bool {
        let mut inert = false;
        let level = self.view(ticks).previewing(preview);
        super::native_hooks::LevelView::new(&level, self.world).native_can_sustain_plant(
            self.dimension,
            state,
            ground,
            ground_pos,
            sustains,
            &mut inert,
        );
        inert
    }
    #[inline]
    fn asks_is_fertile(&self) -> bool {
        self.native_authoritative && mod_api::__private::has_scoped_hook(mod_api::HookId::IsFertile)
    }
    #[inline]
    fn before_is_fertile(&self, state: BlockStateId, pos: BlockPos, fertile: &mut bool) {
        self.native_is_fertile(self.dimension, state, pos, fertile);
    }
    #[inline]
    fn asks_can_be_hydrated(&self, state: BlockStateId) -> bool {
        self.native_authoritative
            && crate::game::api_values::block_state::block_listens(
                mod_api::HookId::CanBeHydrated,
                state,
            )
    }
    #[inline]
    fn hold_farmland_hydration(&self, state: BlockStateId) -> bool {
        mod_api::__private::any_mod_disabled()
            && state.try_into_api().is_ok_and(|state| {
                mod_api::__private::block_hook_inert(mod_api::HookId::CanBeHydrated, state)
            })
    }
    #[inline]
    fn before_can_be_hydrated(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        fluid: FluidState,
        fluid_pos: BlockPos,
        hydrates: &mut Option<bool>,
    ) {
        if let Some(fluid) = crate::game::native_world_read_common::api_fluid_state(fluid) {
            self.native_can_be_hydrated(self.dimension, state, pos, fluid, fluid_pos, hydrates);
        }
    }
    #[inline]
    fn before_may_place_on(
        &self,
        ticks: &Ticks,
        preview: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        may: &mut Option<bool>,
    ) -> bool {
        let mut inert = false;
        let level = self.view(ticks).previewing(preview);
        super::native_hooks::LevelView::new(&level, self.world).native_may_place_on(
            self.dimension,
            state,
            ground,
            ground_pos,
            may,
            &mut inert,
        );
        inert
    }
    #[inline]
    fn is_client_side(&self) -> bool {
        self.client_side
    }
    #[inline]
    fn is_valid_bonemeal_target(&self, pos: BlockPos, state: BlockStateId) -> Option<bool> {
        // Asked on both sides, as vanilla asks `isValidBonemealTarget`: a
        // client predicts the use with it. Only a mod answers.
        let mut valid = None;
        self.native_is_valid_bonemeal_target(self.dimension, pos, state, &mut valid);
        valid
    }
    #[inline]
    fn is_bonemeal_success(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let mut success = false;
        if self.native_authoritative {
            self.native_is_bonemeal_success(self.dimension, pos, state, &mut success);
        }
        success
    }
    #[inline]
    fn perform_bonemeal(&mut self, ticks: &mut Ticks, pos: BlockPos, state: BlockStateId) {
        if self.native_authoritative {
            self.native_perform_bonemeal(ticks, self.dimension, pos, state);
        }
    }
    #[inline]
    fn before_fire_odds(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        odds: &mut Option<(i32, i32)>,
    ) {
        self.answer_fire_odds(state, pos, face, odds);
    }
    #[inline]
    fn before_is_fire_source(&self, state: BlockStateId, pos: BlockPos, source: &mut Option<bool>) {
        self.answer_is_fire_source(state, pos, source);
    }
    #[inline]
    fn before_ignited_by_lava(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        ignites: &mut Option<bool>,
    ) {
        self.answer_ignited_by_lava(state, pos, face, ignites);
    }
    #[inline]
    fn on_caught_fire(
        &mut self,
        ticks: &mut Ticks,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
    ) {
        self.caught_fire(ticks, state, pos, face);
    }
    #[inline]
    fn before_smoke_source(&self, state: BlockStateId, smoke: &mut Option<bool>) {
        self.answer_smoke_source(state, smoke);
    }
    #[inline]
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }
    #[inline]
    fn precipitation(&self, pos: BlockPos) -> Option<sunrise_world::level::Precipitation> {
        self.behaviours.precipitation_info(self, pos)
    }
    #[inline]
    fn is_raining_at(&self, pos: BlockPos) -> bool {
        self.behaviours.is_raining_at(self, pos)
    }
    /// `getEntitiesWithContainerOpen` is not empty: a player or guest has the
    /// container open, or a copper golem has the chest open
    /// (`CopperGolem.hasContainerOpen`).
    #[inline]
    fn container_open(&self, pos: BlockPos) -> bool {
        let key = (pos.x, pos.y, pos.z);
        self.open_containers.contains(&key) || self.golem_chests.contains(&key)
    }
    /// `ContainerOpenersCounter.recheckOpeners` for the block entity at `pos`,
    /// through `ChestBlock.tick`, `EnderChestBlock.tick` and `BarrelBlock.tick`:
    /// the count a recount of the players with that container open finds, and
    /// the count it replaces. Only the block entities that keep a counter and
    /// a scheduled recheck answer; anything else is `None`.
    #[inline]
    fn recheck_container_openers(&mut self, pos: BlockPos) -> Option<(i32, i32)> {
        use sunrise_world::level::classes::Class;
        let data = self.behaviours.data(self.world.block(pos.x, pos.y, pos.z));
        if !(data.is(Class::Chest) || data.is(Class::EnderChest) || data.is(Class::Barrel)) {
            return None;
        }
        Some(
            sunrise_gameplay_common::container_openers::recheck_opener_count(
                self.lid_openers,
                self.open_containers,
                (pos.x, pos.y, pos.z),
            ),
        )
    }

    #[inline]
    fn block_entity_analog_signal(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
    ) -> i32 {
        if let Some(s) = self.registry.state(state)
            && s.name == "minecraft:detector_rail"
        {
            return self.detector_rail_signal(
                pos,
                s.properties.get("powered").is_some_and(|v| v == "true"),
            );
        }
        if let Some(signal) = sunrise_gameplay_common::command_blocks::analog_signal(
            self.world,
            (pos.x, pos.y, pos.z),
        ) {
            return signal;
        }
        if let Some(signal) = sensor_signal(self.world, self.behaviours, state, pos) {
            return signal;
        }
        if let Some((_, comparator)) =
            crate::game::jukebox_common::jukebox_signals(self.world, self.items, pos)
        {
            return comparator;
        }
        if let Some(signal) = crate::game::creaking_common::heart_signal(self.world, pos) {
            return if self.registry.state(state).is_some_and(|s| {
                s.properties
                    .get("creaking_heart_state")
                    .is_some_and(|v| v == "uprooted")
            }) {
                0
            } else {
                signal
            };
        }
        if let Some(signal) =
            sunrise_gameplay_common::lectern::lectern_signal(self.world, self.registry, state, pos)
        {
            return signal;
        }
        if let Some(signal) = sunrise_gameplay_common::hoppers::shelf_signal(
            self.world,
            self.registry,
            state,
            pos,
            direction,
        ) {
            return signal;
        }
        crate::game::machine_containers::container_signal(self, pos)
    }
    #[inline]
    fn item_frame_analog_output(&self, pos: BlockPos, facing: Direction) -> Option<i32> {
        let mut frames = self
            .item_frames
            .iter()
            .filter(|f| f.pos == [pos.x, pos.y, pos.z] && f.facing == facing as u8);
        let frame = frames.next().filter(|_| frames.next().is_none())?;
        // ItemFrame.getAnalogOutput.
        Some(
            frame
                .item
                .as_ref()
                .map_or(0, |_| i32::from(frame.rotation % 8) + 1),
        )
    }
    #[inline]
    fn block_entity_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        native_level_signals::block_entity_signal(
            self.world,
            self.registry,
            self.items,
            self.lid_openers,
            pos,
            state,
        )
    }
    #[inline]
    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let biome = self.world.biome_at(self.biome_zoom, pos.x, pos.y, pos.z)?;
        Some(&self.biomes.get(biome).name)
    }
    #[inline]
    fn world_seed(&self) -> i64 {
        self.seed
    }
    #[inline]
    fn day_time(&self) -> i64 {
        self.world.day_time
    }
    #[inline]
    fn game_rules(&self) -> sunrise_world::level::GameRules {
        self.rules
    }
    #[inline]
    fn difficulty(&self) -> i32 {
        self.difficulty
    }
    /// `ServerLevel.canSpreadFireAround`: a player (not a spectator) closer
    /// than the `fire_spread_radius_around_player` rule to the block's corner
    /// (`ChunkMap.anyPlayerCloseEnoughTo`); -1 spreads everywhere.
    #[inline]
    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        self.player_near_fire(pos)
    }
    #[inline]
    fn increased_fire_burnout(&self, pos: BlockPos) -> bool {
        self.biome_burns_out_fire(pos)
    }
    #[inline]
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }
    #[inline]
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        // `Level.setBlock`: a server level of the Debug preset never changes.
        if self.debug && !self.client_side {
            return None;
        }
        if !self.world.loaded(pos.x, pos.z)
            || pos.y < self.world.bottom_y()
            || pos.y >= self.world.top_y()
        {
            return None;
        }
        let previous = self.world.block(pos.x, pos.y, pos.z);
        self.world
            .set_with_notification(pos.x, pos.y, pos.z, state, false)
            .ok()?;
        Some(previous)
    }
    #[inline]
    fn send_block_updated(&mut self, pos: BlockPos) {
        if (self.client_side || self.tickets.block_ticking(pos.x >> 4, pos.z >> 4))
            && let Some(log) = &mut self.world.block_log
        {
            log.push((pos.x, pos.y, pos.z));
        }
        // Vanilla's full chunk status, not the stricter simulation-ticket gate
        // used by the existing network log above. Independent of log presence.
        if !self.client_side && self.tickets.loaded_ticking(pos.x >> 4, pos.z >> 4) {
            self.world.path_type_block_updated(pos);
        }
    }
    #[inline]
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }
    #[inline]
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    #[inline]
    fn max_y(&self) -> i32 {
        // Exclusive, as LevelAccess::max_y documents.
        self.world.shape.max_y()
    }
    #[inline]
    fn random(&mut self) -> &mut sunrise_core::random::LegacyRandom {
        &mut self.world.random
    }
    #[inline]
    fn game_time(&self) -> i64 {
        self.world.game_time
    }
    #[inline]
    fn block_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) & 15)
    }
    #[inline]
    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }
    #[inline]
    fn sky_darken(&self) -> i32 {
        self.darken
    }
    #[inline]
    fn sun_angle(&self, _pos: BlockPos) -> f32 {
        self.environment
            .sample(&sunrise_level::environment::Sample {
                day_time: self.world.day_time,
                ..Default::default()
            })
            .sun_angle
    }
    #[inline]
    fn is_raining(&self) -> bool {
        self.raining
    }
    #[inline]
    fn is_thundering(&self) -> bool {
        self.thundering
    }
    #[inline]
    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
    /// `Level.isUnobstructed`: the placed state's collision shape against the player.
    /// `Level.isUnobstructed(state, pos, CollisionContext.of(player))`: no
    /// entity that blocks building is in the state's collision shape.
    #[inline]
    fn is_unobstructed(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let player = self.player_collision.as_ref();
        let context = player.map_or_else(
            || CollisionContext::empty(false),
            |player| {
                CollisionContext::for_entity(
                    CollisionEntity {
                        identity: EntityIdentity(0),
                        kind: sunrise_world::collision_context::EntityContextKind::Player,
                        y: player.y,
                        descending: player.descending,
                        main_hand: Some(CollisionItem {
                            item: &player.main_hand_item,
                            count: player.main_hand_count,
                        }),
                    },
                    false,
                )
            },
        );
        let mut scene = collisions::SceneCollisions {
            world: &*self.world,
            binding: self.collision,
            subject: player.map(|player| sunrise_level::collisions::PowderSnowSubject {
                fall_distance: player.fall_distance,
                leather_boots: player.leather_boots,
                ..Default::default()
            }),
        };
        let Ok(shape) = self
            .collision
            .collision_shape_with_context(&mut scene, state, pos, &context)
        else {
            return false;
        };
        if shape.is_empty() {
            return true;
        }
        let shape = shape.moved(sunrise_core::geometry::Vec3::new(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(pos.z),
        ));
        sunrise_world::entity_obstruction::is_unobstructed(
            &mut BuildingObstruction(self.building_blocker_boxes()),
            None,
            &shape,
        )
        .unwrap_or(false)
    }
    /// `ServerLevel.shouldTickBlocksAt`: only the block-ticking ticket range.
    #[inline]
    fn should_tick_blocks_at(&self, chunk: sunrise_core::position::ChunkPos) -> bool {
        self.tickets.block_ticking(chunk.x, chunk.z)
    }
    /// `ServerLevel.isPositionTickingWithEntitiesLoaded`: block ticking, a
    /// ticking chunk, and its entities loaded.
    #[inline]
    fn is_ticking_chunk(&self, chunk: sunrise_core::position::ChunkPos) -> bool {
        self.tickets.block_ticking(chunk.x, chunk.z)
            && ticking_chunk(self.world, chunk.x, chunk.z)
            && !self.world.entities_pending.contains(&(chunk.x, chunk.z))
    }
    /// Entities that trigger pressure plates, tripwires, detector rails and
    /// wooden buttons: the players (the host and the LAN guests), the mobs,
    /// the minecarts and the other entities that don't ignore block triggers,
    /// and for `EntityQuery::Arrows` the arrows.
    #[inline]
    fn count_entities(&self, area: Aabb, query: EntityQuery) -> usize {
        let hit = |(lo, hi): &([f64; 3], [f64; 3])| {
            area.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ))
        };
        let carts = || self.cart_boxes.iter().filter(|b| hit(b)).count();
        let others = || {
            self.crystals
                .iter()
                .map(crate::game::dragon_common::Crystal::bounds)
                .chain(self.trigger_hooks.iter().map(|&(bounds, _)| bounds))
                .chain(self.trigger_extras.iter().copied())
                .filter(|b| area.intersects(*b))
                .count()
        };
        let arrows = || self.arrow_boxes.iter().filter(|b| hit(b)).count();
        let players = || {
            self.player_box
                .iter()
                .chain(&self.guest_boxes)
                .filter(|b| hit(b))
                .count()
        };
        match query {
            EntityQuery::Minecart => carts(),
            EntityQuery::Arrows => arrows(),
            EntityQuery::Mobs => players() + self.count_triggers::<true>(area),
            EntityQuery::Everything => players() + self.count_triggers::<false>(area) + others(),
        }
    }
    #[inline]
    fn emit(&mut self, event: LevelEvent) {
        match event {
            // Block.dropResources draws its loot and spots right away.
            // `popResource` only drops server-side (`!level.isClientSide`):
            // a client's moveBlocks (a piston's block event) skips it.
            LevelEvent::DropResources { pos, state } if !self.client_side => {
                self.drop_resources_now(pos, state)
            }
            LevelEvent::DropResources { .. } => {}
            // `ClientLevel.playSeededSound` plays only when the excluded
            // player is this client's own: the block's `playSound(null, ...)`
            // (a piston's move, a note) reaches it from the server instead.
            LevelEvent::Sound { .. } if self.client_side => {}
            // NoteBlock.triggerEvent plays the head's sound with a null
            // player, so only the server's level plays it; a guest or a
            // remote client hears it from the sound packet.
            LevelEvent::BlockEvent {
                kind: "note_custom_sound",
                ..
            } if self.client_side => {}
            // Block.popResource of the block's own item (tall flowers, flower beds).
            LevelEvent::BlockEvent {
                pos,
                state,
                kind: "pop_block_item",
            } => self.pop_block_item(pos, state),
            // ComposterBlock.extractProduce: bone meal above the composter.
            LevelEvent::Player {
                pos,
                effect: PlayerEffect::DropItem(item),
                ..
            } => self.pop_above(pos, item),
            // Picking a sweet berry bush or cave vines: the harvest loot,
            // popped before the pick sound draws its pitch.
            LevelEvent::Player {
                pos,
                state,
                effect: PlayerEffect::InteractLoot(table),
            } => self.drop_interact_loot(pos, state, table),
            event => self.events.push(event),
        }
    }
    #[inline]
    fn has_mod_block_entity(&self, pos: BlockPos) -> bool {
        // Asked on every block change: no lookup until a mod block entity exists.
        self.world.block_entities.has_mods()
            && matches!(
                self.world.block_entities.get(&(pos.x, pos.y, pos.z)),
                Some(BlockEntity::Mod(_))
            )
    }
    #[inline]
    fn adjust_piston_carries_block_entity(
        &self,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Option<bool> {
        self.piston_carries_block_entity(pos, state)
    }
    #[inline]
    fn carry_block_entity(&mut self, from: BlockPos, to: BlockPos) {
        self.carry_mod_block_entity(from, to);
    }
    #[inline]
    fn land_block_entity(&mut self, pos: BlockPos) -> bool {
        self.land_carried_block_entity(pos)
    }
    #[inline]
    fn discard_carried_block_entity(&mut self, pos: BlockPos) {
        self.world
            .block_entities
            .discard_carried(&(pos.x, pos.y, pos.z));
    }
    /// `ShulkerBoxBlockEntity.isClosed`: the lid state the block-entity tick
    /// copied from `be_anims` (`ShulkerBoxBlock.getBlockSupportShape`).
    #[inline]
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        self.world
            .shulkers
            .get(&[pos.x, pos.y, pos.z])
            .map(|s| s.status == sunrise_level::shulker_box::ShulkerStatus::Closed)
    }
    /// What a mod block entity published for a block that may (`shapes_from_block_entity`).
    #[inline]
    fn published_collision_shape(
        &self,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Option<sunrise_core::geometry::VoxelShape> {
        if !self.collision.publishes_shapes(state) {
            return None;
        }
        sunrise_level::collisions::published_shapes(
            self.world,
            self.registry,
            state,
            [pos.x, pos.y, pos.z],
        )
        .map(|shapes| shapes.collision().clone())
    }
    #[inline]
    fn remove_block_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        if let Some(change) = &mut self.command_block_transition
            && change.preserve(self.world, pos, old)
        {
            // Returning 0 skips remove_entity's side effects once; command blocks have no contents or drops.
            return 0;
        }
        self.remove_entity(pos, old, side_effects)
    }
    #[inline]
    fn load_block_entity(&mut self, pos: BlockPos, mut nbt: sunrise_core::nbt::Compound) {
        use sunrise_core::nbt::Tag;
        for (k, v) in [("x", pos.x), ("y", pos.y), ("z", pos.z)] {
            nbt.insert(k.into(), Tag::Int(v));
        }
        let block = self.world.block(pos.x, pos.y, pos.z);
        let Some(block) = self.registry.state(block) else {
            return;
        };
        let name = &block.name;
        let tag = Tag::Compound(nbt);
        let previous = self.world.block_entities.get(&(pos.x, pos.y, pos.z));
        if let Some((key, entity)) =
            sunrise_sim::save::decode_block_entity_replacing(&tag, name, previous)
        {
            self.world.block_entities.insert(key, entity);
            self.notify_block_entity_load_additional(key);
            if tag.get("LootTable").is_some() {
                self.emit(LevelEvent::ContainerLootTable { pos, tag });
            }
        }
    }
    #[inline]
    fn create_feature_spawner(&mut self, pos: BlockPos, nbt: sunrise_core::nbt::Compound) {
        // Guard at the owner/insert boundary as well as the live feature sink.
        // Existing entries of every class retain the prior no-op behavior.
        if self
            .registry
            .state(self.world.block(pos.x, pos.y, pos.z))
            .is_none_or(|state| state.name != "minecraft:spawner")
            || self
                .world
                .block_entities
                .contains_key(&(pos.x, pos.y, pos.z))
        {
            return;
        }
        self.load_block_entity(pos, nbt);
    }
    #[inline]
    fn set_command_block_powered(&mut self, pos: BlockPos, powered: bool) -> Option<(bool, bool)> {
        sunrise_gameplay_common::command_blocks::set_powered(
            self.world,
            self.registry,
            (pos.x, pos.y, pos.z),
            powered,
        )
    }
    /// `PotentSulfurBlockEntity.resetCountdown`.
    #[inline]
    fn reset_potent_sulfur_countdown(&mut self, pos: BlockPos) {
        if let Some(BlockEntity::PotentSulfur(sulfur)) =
            self.world.block_entities.get_mut(&(pos.x, pos.y, pos.z))
        {
            sulfur.waiting_countdown = -1;
        }
    }
}
