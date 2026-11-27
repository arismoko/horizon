//! Finishing a consumable (`Consumable.onConsume` and the use remainder),
//! its particles and sounds, and `TeleportRandomlyConsumeEffect` (chorus
//! fruit) for players and foxes. The level random stands in for the player's
//! own, as for every other player draw.
use super::Dim;
use std::convert::Infallible;
use sunrise_core::{
    geometry::Vec3,
    position::BlockPos,
    random::{EntityRandom, LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::collision_source::Mover;
use sunrise_gameplay_common::game::item_use_common::{ItemConsumable, USE_ITEM_COMPLETE_EVENT};
use sunrise_items::items::{ConsumeEffect, ItemInfo, ItemStack};
use sunrise_sim::mobs::Who;
use {
    sunrise_core::use_animation::UseAnimation,
    sunrise_world::{
        consumable::EmissionWorld,
        teleport_consume::{TeleportConsume, TeleportConsumeWorld},
    },
};

/// Capture once per effect, not once per attempted destination. If the stack
/// cannot cross the API boundary (for example, an unregistered saved item), skip
/// the listener for this consumption and let the vanilla teleport proceed.
pub(in crate::game::server_side) fn consumed_snapshot(
    stack: &ItemStack,
) -> Option<mod_api::ItemStack> {
    if !mod_api::__private::listened(mod_api::HookId::ItemConsumptionTeleport) {
        return None;
    }
    stack.try_into_api().ok()
}

impl crate::game::ServerGame<'_> {
    /// The consuming player, including the local player outside a guest action.
    /// Before a world is published the handle is stale, but the effect still runs.
    pub(in crate::game::server_side) fn consuming_player_entity(&self) -> mod_api::EntityRef {
        mod_api::EntityRef::new(
            self.common
                .native_lifecycle
                .epoch()
                .unwrap_or(mod_api::WorldEpoch::new(0)),
            if self.server.lan_acting.is_some() {
                self.native_acting_entity()
            } else {
                self.own_entity_id() as u32
            },
        )
    }

    /// `ServerPlayer.completeUsingItem`: the player's own client is told
    /// first, and completes the use itself (eating the food there too).
    pub(in crate::game::server_side) fn send_use_item_complete(&mut self) {
        if self.server.lan_acting.is_some() {
            self.server
                .guest_entity_events
                .push(USE_ITEM_COMPLETE_EVENT);
        }
    }
    /// Server-side `Consumable.emitParticlesAndSounds`.
    pub(in crate::game) fn server_emit_consume(
        &mut self,
        consumable: &ItemConsumable,
        stack: &ItemStack,
        count: i32,
    ) {
        if self.common.player.is_some() {
            let Ok(()) = consumable.emit(&mut Emitter(self), (), stack.clone(), count);
        }
    }
    /// `ItemStack.applyAfterUseComponentSideEffects` uses the stack captured
    /// before the item handler, even when that handler changes the hand.
    pub(in crate::game) fn apply_after_use_components(&mut self, stack: &ItemStack) {
        self.apply_use_remainder(stack);
        self.apply_use_cooldown(stack);
    }
    pub(in crate::game::server_side) fn apply_use_remainder(&mut self, used: &ItemStack) {
        if self.common.creative || self.common.held().map_or(0, |stack| stack.count) >= used.count {
            return;
        }
        if let Some(remainder) =
            sunrise_sim::item_sync::use_remainder(used, self.items().get(&used.item))
        {
            if self.common.held().is_none() {
                *self.common.held_mut() = Some(remainder);
            } else if let Some(rest) = self.add_stack_to_inventory(remainder) {
                self.player_drop(rest);
            }
        }
    }
    pub(in crate::game::server_side) fn feet(&self) -> Option<[f64; 3]> {
        self.common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
    }
    /// `TeleportRandomlyConsumeEffect.apply` for the local player.
    pub fn teleport_randomly(&mut self, teleport: &TeleportConsume, consumed: &ItemStack) -> bool {
        if self.common.player.is_none() {
            return false;
        }
        let dim = self.common.dim;
        let consumed = consumed_snapshot(consumed);
        let Ok(result) = teleport.apply(&mut Teleporter(self, consumed.as_ref()), dim);
        result
    }
    /// Apply the paused fox item use with the full host, then leave its tick
    /// continuation to the owning mob loop. Callback writes and apply share a transaction.
    pub fn apply_fox_consumption(
        &mut self,
        decision: &sunrise_sim::mobs::fox::Consumption,
    ) -> bool {
        if !self.common.mobs.iter().any(|m| decision.still_valid(m)) {
            return false;
        }
        let Some(consumer) = self.entity(decision.mob).and_then(|e| self.living(e)) else {
            return false;
        };
        let Some(consumable) = self.consumable(&decision.stack) else {
            return false;
        };
        let failures = mod_api::runtime::isolated_failures();
        let result: Result<bool, ()> = self.storage_transaction(|game, _| {
            {
                let mut mobs = game.mobs.lift();
                if let Some(mob) = mobs.iter_mut().find(|m| m.id == decision.mob) {
                    let mut events = Vec::new();
                    sunrise_sim::mobs::fox::consumed_before_effects(
                        &game.mob_world_base(&[]),
                        mob,
                        &mut events,
                        &decision.stack,
                    );
                    game.mobs
                        .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
                    if game.handle_mob_events(events).is_err() {
                        return if mod_api::runtime::isolated_failures() != failures {
                            Err(())
                        } else {
                            Ok(false)
                        };
                    }
                } else {
                    game.mobs
                        .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
                    return Ok(false);
                }
                let can_finish = game.apply_consumable_effects(
                    &decision.stack,
                    &consumable.effects,
                    Some((consumer, MobConsumption::Fox(decision))),
                );
                if mod_api::runtime::isolated_failures() != failures {
                    return Err(());
                }
                // A refusal (for example, a listener moved the fox under the
                // pending teleport) still invalidates the original finish.
                if !can_finish {
                    return Ok(false);
                }
                if let Some(mob) = game
                    .mobs
                    .iter()
                    .find(|m| m.id == decision.mob && m.alive() && !m.removed)
                {
                    let event = sunrise_sim::mobs::MobEvent::GameEvent {
                        event: if consumable.animation == UseAnimation::Drink {
                            "minecraft:drink"
                        } else {
                            "minecraft:eat"
                        },
                        at: [mob.position.x, mob.position.y, mob.position.z],
                        source: mob.id,
                        state: sunrise_world::registry::BlockStateId(0),
                    };
                    if game.handle_mob_events(vec![event]).is_err() {
                        return if mod_api::runtime::isolated_failures() != failures {
                            Err(())
                        } else {
                            Ok(false)
                        };
                    }
                }
            }
            if mod_api::runtime::isolated_failures() != failures {
                return Err(());
            }
            let Some(m) = game.common.mobs.iter_mut().find(|m| m.id == decision.mob) else {
                return Ok(false);
            };
            let position = m.position;
            let info = game.common.res.items().get(&decision.stack.item);
            Ok(decision.finish(m, position, info))
        });
        result.unwrap_or(false)
    }

    /// Run the original user's effective effect list, never re-reading its hand.
    /// If a built-in can no longer act, continue the list but refuse the finish.
    pub(in crate::game::server_side) fn apply_consumable_effects(
        &mut self,
        stack: &ItemStack,
        effects: &[ConsumeEffect],
        mob_use: Option<(mod_api::LivingRef, MobConsumption<'_>)>,
    ) -> bool {
        if !self.common.is_logical_server() || effects.is_empty() {
            return true;
        }
        let Some((consumer, usage)) = mob_use else {
            let user = self.consuming_player_entity();
            for effect in effects {
                self.apply_consume_effect(stack, effect, None, user);
            }
            return true;
        };
        let id = consumer.entity().id();
        let dim = self.common.dim;
        let consumed = std::cell::OnceCell::new();
        let mut can_finish = true;
        for effect in effects {
            match effect {
                ConsumeEffect::PlaySound(_) | ConsumeEffect::Native(_) => {
                    can_finish &= self
                        .apply_consume_effect(stack, effect, Some(consumer), consumer.entity())
                        .is_some();
                }
                ConsumeEffect::TeleportRandomly(teleport) => {
                    let consumed = consumed.get_or_init(|| consumed_snapshot(stack));
                    let mut actor = MobTeleporter {
                        game: self,
                        consumer,
                        id,
                        usage,
                        dim,
                        consumed: consumed.as_ref(),
                    };
                    can_finish &= teleport.apply(&mut actor, dim).is_ok();
                }
                ConsumeEffect::ApplyEffects { .. }
                | ConsumeEffect::ClearAllEffects
                | ConsumeEffect::RemoveEffects(_) => {
                    if self
                        .common
                        .mobs
                        .iter()
                        .any(|m| m.id == id && m.alive() && !m.removed)
                    {
                        can_finish &= self
                            .apply_consume_effect(stack, effect, Some(consumer), consumer.entity())
                            .is_some();
                    }
                }
            }
        }
        can_finish
    }

    /// `DeathProtection.applyEffects` for a mob saved by a `death_protection`
    /// item: runs every `ConsumeEffect` in list order on the owning `Game` so
    /// `MobEffectEvent.Remove`, `Applicable`, `Added` and
    /// `MobEffectPart::on_effect_added` all fire.
    pub(in crate::game) fn apply_mob_death_protection_effects(
        &mut self,
        id: u32,
        stack: &ItemStack,
        effects: &[ConsumeEffect],
    ) {
        let Some(consumer) = self.entity(id).and_then(|entity| self.living(entity)) else {
            return;
        };
        self.apply_consumable_effects(
            stack,
            effects,
            Some((consumer, MobConsumption::DeathProtection)),
        );
    }

    /// Specialized player drinks have already applied their built-in component listener.
    pub(in crate::game::server_side) fn apply_drink_effects(
        &mut self,
        stack: &ItemStack,
        consumable: &ItemConsumable,
    ) {
        self.apply_consumable_effects(stack, &consumable.effects, None);
    }

    /// A witch or trader applied its ordinary drink behavior during AI. The
    /// owning game now runs the list with the living host, before clearing the hand.
    pub(in crate::game::server_side) fn finish_mob_drink(&mut self, id: u32, stack: &ItemStack) {
        let usage = MobConsumption::Drink(stack);
        if !self
            .common
            .mobs
            .iter()
            .any(|mob| mob.id == id && usage.still_valid(mob, mob.position))
        {
            return;
        }
        let Some(consumer) = self.entity(id).and_then(|entity| self.living(entity)) else {
            return;
        };
        let consumable = self.consumable(stack);
        let failures = mod_api::runtime::isolated_failures();
        let result: Result<(), ()> = self.storage_transaction(|game, _| {
            if stack.potion().unwrap_or(&stack.item) == "minecraft:healing" {
                if let Some(mob) = game
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                {
                    mob.heal(
                        &game.common.loop_context,
                        &|| crate::game::entity_server::hit_reads_of!(game),
                        4.0,
                    );
                }
            } else if let Some(effect) = sunrise_sim::mobs::ai::drink_potion_effect(stack) {
                game.add_mob_effect(id, effect, None);
            }
            if let Some(consumable) = &consumable
                && !game.apply_consumable_effects(
                    stack,
                    &consumable.effects,
                    Some((consumer, usage)),
                )
            {
                return if mod_api::runtime::isolated_failures() != failures {
                    Err(())
                } else {
                    Ok(())
                };
            }
            if mod_api::runtime::isolated_failures() != failures {
                return Err(());
            }
            if let Some(mob) = game
                .mobs
                .iter_mut()
                .find(|mob| mob.id == id && usage.still_valid(mob, mob.position))
            {
                mob.extra.set_main_hand_stack(None);
            }
            Ok(())
        });
        let _ = result;
    }

    /// `LivingEntity.randomTeleport(x, y, z, true, avoid)`: down to the first
    /// block entities can teleport onto, inside the border, in a loaded chunk.
    pub fn random_teleport(&mut self, target: Vec3, avoid: Option<&str>) -> bool {
        let [min_x, min_z, max_x, max_z] = self.common.border.bounds();
        // WorldBorder.clampVec3ToBound (Mth.clamp, 1.0E-5F inside the far edge).
        let clamp = |v: f64, low: f64, high: f64| if v < low { low } else { v.min(high) };
        let x = clamp(target.x, min_x, max_x - f64::from(1.0e-5_f32));
        let z = clamp(target.z, min_z, max_z - f64::from(1.0e-5_f32));
        let mut y = target.y;
        let mut pos = BlockPos::containing(Vec3::new(x, y, z));
        if !self.common.world.loaded(pos.x, pos.z) {
            return false;
        }
        while pos.y > self.common.world.shape.min_y {
            pos.y -= 1;
            let state = self.common.world.block(pos.x, pos.y, pos.z);
            if self.state_in_tag("minecraft:entities_can_teleport_to", state) {
                return self.check_position_and_teleport(Vec3::new(x, y, z), state, avoid);
            }
            y -= 1.0;
        }
        false
    }
    /// `LivingEntity.checkPositionAndTeleport`, then `ServerPlayer.teleportTo`
    /// (the motion and rotation kept) and entity event 46.
    pub(in crate::game::server_side) fn check_position_and_teleport(
        &mut self,
        at: Vec3,
        ground: sunrise_world::registry::BlockStateId,
        avoid: Option<&str>,
    ) -> bool {
        if avoid.is_some_and(|tag| self.state_in_tag(tag, ground)) {
            return false;
        }
        let entities = self.entity_collisions(Mover::Player);
        let Some(player) = self.common.player.as_mut() else {
            return false;
        };
        let bounds = player.dimensions.bounds(at);
        let side = server_game_side!(self);
        let free = sunrise_sim::motion::MotionWorld {
            side: &side,
            entities: &entities,
            player,
            scene: &mut self.common.world,
            assets: &self.common.pack.assets,
            collision_binding: self.common.collision,
            resources: self.common.motion,
            health: &self.common.health_resources,
            loop_context: &self.common.loop_context,
            damage_hooks: &self.common.damage_hooks,
            tag_items: Some(server_world_items!(self)),
            block_registries: Some(&self.common.pack.block_registries),
        }
        .no_player_collision(bounds)
        .unwrap_or(false);
        // LevelReader.containsAnyLiquid and findBlocksIn: the blocks from the
        // box's minimum corner to the one holding its maximum corner.
        let (lo, hi) = (
            BlockPos::containing(bounds.min_position()),
            BlockPos::containing(bounds.max_position()),
        );
        let blocks = (lo.x..=hi.x)
            .flat_map(|x| (lo.y..=hi.y).flat_map(move |y| (lo.z..=hi.z).map(move |z| (x, y, z))));
        let states: Vec<_> = blocks
            .map(|(x, y, z)| self.common.world.block(x, y, z))
            .collect();
        let liquid = states
            .iter()
            .any(|&s| self.common.res.facts().get(s).fluid != sunrise_items::blocks::Fluid::None);
        if !free
            || liquid
            || avoid.is_some_and(|tag| states.iter().any(|&s| self.state_in_tag(tag, s)))
        {
            return false;
        }
        if let Some(p) = self.common.player.as_mut() {
            p.position = at;
        }
        // The position packet moves the old position along (absolute xyz).
        self.common.previous_position = at;
        // `broadcastEntityEvent(46)` after the teleport. A server-thread window
        // is a remote client, so the packet is what draws the particles there.
        if let Some(id) = self.server.lan_acting_entity {
            self.lan_entity_event(id as i32, 46, [at.x, at.y, at.z]);
        }
        true
    }
    pub(in crate::game::server_side) fn state_in_tag(
        &self,
        tag: &str,
        state: sunrise_world::registry::BlockStateId,
    ) -> bool {
        let name = &self.common.res.facts().get(state).name;
        self.common
            .res
            .items()
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(&**name))
    }
    /// `LivingEntity.stopRiding`.
    pub fn stop_riding(&mut self) {
        let Some(id) = self.common.riding else {
            return;
        };
        let removed_y = self
            .common
            .persist
            .removed_vehicles
            .get(&id)
            .map(|vehicle| vehicle.position().y)
            .or_else(|| {
                use super::commands::Target;
                let boat = Target::Boat(id);
                let cart = Target::Cart(id);
                if self.common.persist.removals.contains(&boat)
                    || self.common.persist.destroying_vehicles.contains(&boat)
                {
                    self.common
                        .boats
                        .iter()
                        .find(|vehicle| vehicle.id == id)
                        .map(|vehicle| vehicle.position.y)
                } else if self.common.persist.removals.contains(&cart)
                    || self.common.persist.destroying_vehicles.contains(&cart)
                {
                    self.common
                        .carts
                        .iter()
                        .find(|vehicle| vehicle.id == id)
                        .map(|vehicle| vehicle.position.y)
                } else {
                    None
                }
            });
        if let Some(y) = removed_y {
            self.stop_riding_from_removed_vehicle(id, y);
            return;
        }
        if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
            self.dismount(i);
        } else if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
            self.dismount_cart(i);
        } else if let Some(c) = self.common.cushions.iter().find(|c| c.id == id).cloned() {
            self.get_up_from_cushion(&c);
        } else if self.seat(id).is_some() {
            self.get_off_seat();
        } else {
            self.dismount_mount();
        }
    }

    pub(in crate::game::server_side) fn stop_riding_from_removed_vehicle(
        &mut self,
        vehicle: u32,
        y: f64,
    ) {
        if self.native_may_leave(vehicle) && self.common.riding == Some(vehicle) {
            self.common.dismount_removed_vehicle(y);
            self.release_removed_vehicle(vehicle);
        }
    }

    /// `LivingEntity.completeUsingItem` for a consumable: `Consumable.onConsume`,
    /// then `ItemStack.applyAfterUseComponentSideEffects`' use remainder.
    pub fn finish_consuming(
        &mut self,
        info: &ItemInfo,
        consumable: &ItemConsumable,
        stack: &ItemStack,
    ) {
        let slot = self.common.held_slot();
        self.emit_consume(consumable, stack, 16);
        // ServerPlayer: the used statistic and the consume_item trigger.
        self.award_stat("used", &stack.item, 1);
        self.trigger_item("minecraft:consume_item", stack);
        // FoodProperties.onConsume, the player's one ConsumableListener.
        if let Some((nutrition, saturation)) = sunrise_sim::item_sync::food(stack, info.food)
            && let Some(player) = self.common.player.as_mut()
        {
            let at = [player.position.x, player.position.y, player.position.z];
            let eat_pitch = player.random.triangle_float(1.0, 0.4);
            // FoodData.eat(nutrition, saturation)
            let food = &mut player.health.food;
            food.food = (food.food + nutrition as i32).clamp(0, 20);
            food.saturation = (food.saturation + saturation).clamp(0.0, food.food as f32);
            // Mth.randomBetween(random, 0.9, 1.0)
            let pitch = player.random.next_float() * (1.0f32 - 0.9f32) + 0.9;
            self.server_play_sound(&consumable.sound, at, 1.0, eat_pitch);
            self.server_play_sound("minecraft:entity.player.burp", at, 0.5, pitch);
        }
        // SuspiciousStewEffects.onConsume, a ConsumableListener: each entry is
        // added as a `MobEffectInstance(effect, duration)` (amplifier 0).
        if let Some(data) = stack.extra.as_deref() {
            for (id, duration) in &data.stew_effects {
                if let Some(def) = sunrise_level::brewing::effect(id) {
                    self.add_player_effect(sunrise_level::effects::EffectInstance::new(
                        def.id, *duration, 0,
                    ));
                }
            }
        }
        // Consumable.onConsume applies these entries on the logical server only;
        // the client's listener, particles and sounds above still run there.
        self.apply_consumable_effects(stack, &consumable.effects, None);
        let event = if consumable.animation == UseAnimation::Drink {
            "minecraft:drink"
        } else {
            "minecraft:eat"
        };
        self.post_entity_event(event, Who::Player);
        // ItemStack.consume, then UseRemainder.convertIntoRemainder: nothing
        // with infinite materials, the hand when emptied, else the inventory
        // (ServerPlayer.handleExtraItemsCreatedOnUse) or a drop. A listener may
        // replace the stack or change the selected slot; neither is this use.
        if self.common.held_slot() != slot || self.common.stack_in(slot) != Some(stack) {
            return;
        }
        self.consume_held();
        self.apply_use_remainder(stack);
    }
}

/// `Consumable.emitParticlesAndSounds` on the server.
pub(in crate::game::server_side) struct Emitter<'a, 'g>(
    pub(in crate::game::server_side) &'a mut crate::game::ServerGame<'g>,
);
impl EmissionWorld for Emitter<'_, '_> {
    type Error = Infallible;
    type Stack = ItemStack;
    type Sound = String;
    type Random = ();
    type Source = EntityRandom;
    fn random_source(&mut self, (): ()) -> &mut EntityRandom {
        &mut self.0.player.as_mut().expect("emitting player").random
    }
    /// `LivingEntity.spawnItemParticles`.
    fn spawn_item_particles(&mut self, _: ItemStack, count: i32) -> Result<(), Infallible> {
        let Some(position) = self.0.player.as_ref().map(|player| player.position) else {
            return Ok(());
        };
        let eye_y = self.0.eye_position().map_or(position.y, |eye| eye[1]);
        let (pitch, yaw) = (self.0.pitch, self.0.yaw);
        // The headless server still advances the player's random source for each crumb.
        sunrise_level::particle_options::item_use_particles(
            self.random_source(()),
            pitch,
            yaw,
            [position.x, position.y, position.z],
            eye_y,
            count.max(0) as u32,
            |_, _| {},
        );
        Ok(())
    }
    fn overrides_consume_sound(&self) -> bool {
        false
    }
    fn consume_sound(&mut self, _: ItemStack) -> Result<String, Infallible> {
        unreachable!("players play the consumable's own sound")
    }
    /// `Player.playSound`: at the player's position, for everyone but the
    /// player, whose own client plays it (`level.playSound(player, ...)`):
    /// a guest's server turn leaves the guest out.
    fn play_sound(&mut self, sound: String, volume: f32, pitch: f32) -> Result<(), Infallible> {
        let Some(at) = self.0.feet() else {
            return Ok(());
        };
        if self.0.server.lan_acting.is_some() {
            self.0
                .server
                .guest_player_sounds
                .push((sound, at, volume, pitch));
        } else {
            self.0.server_play_sound(&sound, at, volume, pitch);
        }
        Ok(())
    }
}

/// The local player as `TeleportRandomlyConsumeEffect`'s user.
pub(in crate::game) struct Teleporter<'a, 'g>(
    pub(in crate::game) &'a mut crate::game::ServerGame<'g>,
    pub(in crate::game) Option<&'a mod_api::ItemStack>,
);
impl Teleporter<'_, '_> {
    pub(in crate::game::server_side) fn at(&self) -> Vec3 {
        self.0.player.as_ref().map_or(Vec3::ZERO, |p| p.position)
    }
}
impl TeleportConsumeWorld for Teleporter<'_, '_> {
    type Error = Infallible;
    type Level = Dim;
    type Random = ();
    type Source = EntityRandom;
    fn position_field(&self) -> Vec3 {
        self.at()
    }
    fn get_random(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
    fn random_source(&mut self, (): ()) -> &mut EntityRandom {
        &mut self.0.player.as_mut().expect("teleporting player").random
    }
    fn min_y(&mut self, level: Dim) -> Result<i32, Infallible> {
        Ok(self.0.server.level_shapes.get(level.id()).min_y)
    }
    fn logical_height(&mut self, level: Dim) -> Result<i32, Infallible> {
        Ok(self.0.server.level_shapes.get(level.id()).logical_height)
    }
    fn is_passenger(&mut self) -> Result<bool, Infallible> {
        Ok(self.0.riding.is_some())
    }
    fn stop_riding(&mut self) -> Result<(), Infallible> {
        self.0.stop_riding();
        Ok(())
    }
    fn position(&mut self) -> Result<Vec3, Infallible> {
        Ok(self.at())
    }
    fn teleport_allowed(&mut self, target: &mut Vec3) -> Result<bool, Infallible> {
        let Some(consumed) = self.1 else {
            return Ok(true);
        };
        let Some(player) = self.0.native_acting_player() else {
            return Ok(true);
        };
        let from = self.at();
        let dimension = self.0.dim;
        let mut to = [target.x, target.y, target.z];
        let mut cancel = false;
        self.0.native_item_consumption_teleport(
            mod_api::LivingRef::new(player.entity()),
            consumed,
            [from.x, from.y, from.z],
            &mut to,
            &mut cancel,
        );
        if self.0.dim != dimension
            || self
                .0
                .player
                .as_ref()
                .is_none_or(|p| p.position != from || p.health.dead_or_dying())
            || self.0.sleeping.is_some()
            || self.0.riding.is_some()
            || self.0.native_acting_player() != Some(player)
        {
            return Ok(false);
        }
        // Keep the mod's spot inside the level as vanilla's own draw is (a
        // huge y would walk down a long way); one that is not a number keeps
        // the draw.
        if to.iter().all(|v| v.is_finite()) {
            let shape = self.0.server.level_shapes.get(self.0.dim.id());
            let high = shape.min_y + shape.logical_height - 1;
            let y = to[1].clamp(f64::from(shape.min_y), f64::from(high));
            *target = Vec3::new(to[0], y, to[2]);
        }
        Ok(!cancel)
    }
    fn random_teleport(&mut self, target: Vec3) -> Result<bool, Infallible> {
        Ok(self
            .0
            .random_teleport(target, Some("minecraft:consumable_does_not_teleport_to")))
    }
    fn teleport_event(&mut self, _: Dim, old: Vec3) -> Result<(), Infallible> {
        let ctx = self.0.who_context(Some(Who::Player), None);
        self.0
            .post_game_event("minecraft:teleport", [old.x, old.y, old.z], ctx);
        Ok(())
    }
    fn is_fox(&self) -> bool {
        false
    }
    fn teleport_sound(&mut self, _: Dim, at: Vec3, _: bool) -> Result<(), Infallible> {
        self.0.server_play_sound(
            "minecraft:item.chorus_fruit.teleport",
            [at.x, at.y, at.z],
            1.0,
            1.0,
        );
        Ok(())
    }
    fn block_position(&mut self) -> Result<BlockPos, Infallible> {
        Ok(BlockPos::containing(self.at()))
    }
    fn directional_particles(
        &mut self,
        _: Dim,
        origin: BlockPos,
        offset: i32,
    ) -> Result<(), Infallible> {
        self.0
            .server_level_event_at(2017, [origin.x, origin.y, origin.z], offset);
        Ok(())
    }
    fn reset_fall_distance(&mut self) -> Result<(), Infallible> {
        if let Some(p) = self.0.player.as_mut() {
            p.fall_distance = 0.0;
        }
        Ok(())
    }
    fn reset_impulse(&mut self) -> Result<(), Infallible> {
        if let Some(p) = self.0.player.as_mut() {
            p.impulse.reset();
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(in crate::game) enum MobConsumption<'a> {
    Fox(&'a sunrise_sim::mobs::fox::Consumption),
    Drink(&'a ItemStack),
    DeathProtection,
}
impl MobConsumption<'_> {
    pub(in crate::game) fn still_valid(self, mob: &sunrise_sim::mobs::Mob, from: Vec3) -> bool {
        match self {
            Self::Fox(decision) => decision.still_valid_at(mob, from),
            Self::Drink(stack) => {
                mob.alive()
                    && !mob.removed
                    && mob.position == from
                    && mob.extra.main_hand_stack() == Some(stack)
            }
            Self::DeathProtection => mob.alive() && !mob.removed && mob.position == from,
        }
    }
}

/// A paused mob as the same `TeleportRandomlyConsumeEffect` user as a player.
pub(in crate::game) struct MobTeleporter<'a, 'g> {
    pub(in crate::game) game: &'a mut crate::game::ServerGame<'g>,
    pub(in crate::game) consumer: mod_api::LivingRef,
    pub(in crate::game) id: u32,
    pub(in crate::game) usage: MobConsumption<'a>,
    pub(in crate::game) dim: Dim,
    pub(in crate::game) consumed: Option<&'a mod_api::ItemStack>,
}
impl MobTeleporter<'_, '_> {
    pub(in crate::game) fn at(&self) -> Result<Vec3, ()> {
        self.game
            .mobs
            .iter()
            .find(|m| m.id == self.id && m.alive() && !m.removed)
            .map(|m| m.position)
            .ok_or(())
    }
}

impl TeleportConsumeWorld for MobTeleporter<'_, '_> {
    type Error = ();
    type Level = Dim;
    type Random = u32;
    type Source = LegacyRandom;
    fn position_field(&self) -> Vec3 {
        // get_random immediately refuses an absent actor before any draw or apply.
        self.at().unwrap_or_default()
    }
    fn get_random(&mut self) -> Result<u32, ()> {
        self.at()?;
        Ok(self.id)
    }
    fn random_source(&mut self, id: u32) -> &mut LegacyRandom {
        &mut self
            .game
            .mobs
            .iter_mut()
            .find(|m| m.id == id)
            .expect("get_random validated the actor without an intervening callback")
            .random
    }
    fn min_y(&mut self, _: Dim) -> Result<i32, ()> {
        Ok(self.game.world.shape.min_y)
    }
    fn logical_height(&mut self, _: Dim) -> Result<i32, ()> {
        Ok(self.game.world.shape.logical_height)
    }
    fn is_passenger(&mut self) -> Result<bool, ()> {
        self.game
            .mobs
            .iter()
            .find(|m| m.id == self.id)
            .map(|m| m.extra.passenger)
            .ok_or(())
    }
    fn stop_riding(&mut self) -> Result<(), ()> {
        self.game.dismount_mob(self.id);
        Ok(())
    }
    fn position(&mut self) -> Result<Vec3, ()> {
        self.at()
    }
    fn teleport_allowed(&mut self, target: &mut Vec3) -> Result<bool, ()> {
        let from = self.at()?;
        let mut to = [target.x, target.y, target.z];
        let mut cancel = false;
        if let Some(consumed) = self.consumed {
            self.game.native_item_consumption_teleport(
                self.consumer,
                consumed,
                [from.x, from.y, from.z],
                &mut to,
                &mut cancel,
            );
        }
        if self.game.dim != self.dim
            || !matches!(self.game.native_living_target(self.consumer),
                Some(super::commands_common::Target::Mob(id)) if id == self.id)
            || !self
                .game
                .mobs
                .iter()
                .any(|m| m.id == self.id && self.usage.still_valid(m, from))
        {
            return Err(());
        }
        if to.iter().all(|v| v.is_finite()) {
            let shape = self.game.world.shape;
            let high = shape.min_y + shape.logical_height - 1;
            *target = Vec3::new(
                to[0],
                to[1].clamp(f64::from(shape.min_y), f64::from(high)),
                to[2],
            );
        }
        Ok(!cancel)
    }
    fn random_teleport(&mut self, at: Vec3) -> Result<bool, ()> {
        Ok(self.game.living_random_teleport(
            self.consumer,
            [at.x, at.y, at.z],
            Some("minecraft:consumable_does_not_teleport_to"),
        ))
    }
    fn teleport_event(&mut self, _: Dim, old: Vec3) -> Result<(), ()> {
        let ctx = self.game.who_context(Some(Who::Mob(self.id)), None);
        self.game
            .post_game_event("minecraft:teleport", [old.x, old.y, old.z], ctx);
        Ok(())
    }
    fn is_fox(&self) -> bool {
        matches!(self.usage, MobConsumption::Fox(_))
    }
    fn teleport_sound(&mut self, _: Dim, at: Vec3, fox: bool) -> Result<(), ()> {
        self.game.server_play_sound(
            if fox {
                "minecraft:entity.fox.teleport"
            } else {
                "minecraft:item.chorus_fruit.teleport"
            },
            [at.x, at.y, at.z],
            1.0,
            1.0,
        );
        Ok(())
    }
    fn block_position(&mut self) -> Result<BlockPos, ()> {
        Ok(BlockPos::containing(self.at()?))
    }
    fn directional_particles(&mut self, _: Dim, origin: BlockPos, offset: i32) -> Result<(), ()> {
        self.game
            .server_level_event_at(2017, [origin.x, origin.y, origin.z], offset);
        Ok(())
    }
    fn reset_fall_distance(&mut self) -> Result<(), ()> {
        let m = self
            .game
            .mobs
            .iter_mut()
            .find(|m| m.id == self.id)
            .ok_or(())?;
        m.fall_distance = 0.0;
        Ok(())
    }
    fn reset_impulse(&mut self) -> Result<(), ()> {
        // The existing mob model has no separate CurrentImpulseContext state.
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod teleport_hook_tests {
    use mod_api::{Flow, Hooks, ItemConsumptionTeleportEvent, ServerHost};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    pub struct ConsumptionProbe(pub Arc<Mutex<Vec<mod_api::ItemStack>>>, pub Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for ConsumptionProbe {
        fn item_consumption_teleport(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ItemConsumptionTeleportEvent<'_>,
        ) -> Flow {
            assert_eq!(
                event.consumer.entity(),
                host.players().acting_player().unwrap().entity()
            );
            self.0.lock().unwrap().push(event.consumed.clone());
            assert_eq!(event.consumed.count(), 7);
            assert_eq!(event.consumed.damage(), 3);
            assert!(event.consumed.components().is_some());
            if self.1.load(Ordering::SeqCst) {
                assert!(
                    event
                        .consumer
                        .entity()
                        .teleport_to(host, [3.0, 70.0, 3.0], None)
                );
                return Flow::Continue;
            }
            Flow::Cancel
        }
    }

    #[cfg(test)]
    #[test]
    fn unregistered_consumed_stack_skips_the_snapshot() {
        use super::consumed_snapshot;
        use mod_api::ModEntry;
        use sunrise_items::items::ItemStack;

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "consumed-stack-proof",
            Box::new(ConsumptionProbe(
                Arc::clone(&heard),
                Arc::new(AtomicBool::new(false)),
            )),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            assert!(mod_api::__private::listened(
                mod_api::HookId::ItemConsumptionTeleport
            ));
            let stack = ItemStack::new("missing:saved_fruit", 1);
            assert!(consumed_snapshot(&stack).is_none());
            assert_eq!(stack.item.as_ref(), "missing:saved_fruit");
            assert_eq!(stack.count, 1);
            assert!(heard.lock().unwrap().is_empty());
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
