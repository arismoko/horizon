use super::*;
use sunrise_gameplay_common::game::item_abilities::{ItemAbilitiesGameExt1 as _, ability};

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Entity.isColliding(pos, state)` for the local player: the state's own
    /// collision shape, at the player's pose, against the player's box.
    pub(in crate::game) fn block_meets_player(&self, pos: [i32; 3], state: BlockStateId) -> bool {
        let Some(player) = self.common.player.as_ref() else {
            return false;
        };
        let entity = player.bounds();
        let context = sunrise_world::collision_context::CollisionContext::for_entity(
            sunrise_world::collision_context::CollisionEntity {
                identity: sunrise_world::entity_obstruction::EntityIdentity(0),
                kind: sunrise_world::collision_context::EntityContextKind::Player,
                y: player.position.y,
                descending: player.shift_key_down,
                main_hand: None,
            },
            false,
        );
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        match self
            .common
            .collision
            .collision_shape_with_context(&mut scene, state, at, &context)
        {
            Ok(shape) => shape_meets(&shape, pos, entity),
            Err(_) => false,
        }
    }
    /// `InteractionResult.CONSUME` for this click: the block keeps it and does nothing, so the
    /// item in hand never gets it either (an empty flower pot, a pressed button). For the
    /// guide's look, as `block_takes_click`.
    pub fn block_swallows_click(
        &mut self,
        hit: &super::super::Hit,
        held: Option<&ItemStack>,
    ) -> bool {
        matches!(
            self.remote_block_reacts(hit, held, false).0,
            InteractionResult::Consume
        )
    }
    /// Whether a main-hand right click with `held` would end on the block hit, for the gameplay
    /// guide: what `remote_block_reacts` answers on its dry level, by the templates' rules alone,
    /// so looking asks no mod's placement or survival hook.
    pub fn block_takes_click(&mut self, hit: &super::super::Hit, held: Option<&ItemStack>) -> bool {
        self.remote_block_reacts(hit, held, false)
            .0
            .consumes_action()
    }
    /// The idle controller guide cannot know what these Parts will decide on a click.
    pub fn can_hint_block_placement(stack: &ItemStack) -> bool {
        use mod_api::{__private::item_listens, HookId};
        !item_listens(HookId::PlacementContext, &stack.item)
            && !item_listens(HookId::MustSurvive, &stack.item)
    }
    /// The shared dry-level boundary for a normal block use or the empty
    /// method requested by a canceled `BLOCK` phase's `TryWithEmptyHand`.
    /// The `useItemOn` answers a client level gives without changing a block, for either hand:
    /// - VaultBlock: something in the hand on an ACTIVE vault is SUCCESS_SERVER (no swing here);
    /// - RedStoneOreBlock: SUCCESS, unless a block item goes in the cell beside it
    ///   (`BlockPlaceContext.canPlace`), PASS;
    /// - PumpkinBlock: shears are SUCCESS (only the server carves it);
    /// - WeatheringCopperGolemStatueBlock: an axe on an unaffected statue is SUCCESS (the server
    ///   lets the golem out).
    pub(in crate::game::client_side) fn client_block_answer(
        &self,
        hit: &sunrise_gameplay_common::game::Hit,
        held: Option<&ItemStack>,
    ) -> Option<InteractionResult> {
        if self.vault_takes_click(hit.pos, held) {
            return Some(InteractionResult::Consume);
        }
        let name = self.block_name(hit.pos);
        match &*name {
            "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore" => {
                let beside = [0, 1, 2].map(|i| hit.pos[i] + hit.face[i]);
                let takes = self
                    .common
                    .res
                    .facts()
                    .get(self.common.world.block(beside[0], beside[1], beside[2]))
                    .replaceable;
                let places = held
                    .is_some_and(|s| self.common.res.items().block_items.contains_key(&*s.item));
                Some(if places && takes {
                    InteractionResult::Pass
                } else {
                    InteractionResult::Success
                })
            }
            "minecraft:pumpkin" => held
                .is_some_and(|s| self.stack_shears(s, ability::SHEARS_CARVE))
                .then_some(InteractionResult::Success),
            "minecraft:copper_golem_statue" => held
                .is_some_and(|s| self.item_in_tag("minecraft:axes", &s.item))
                .then_some(InteractionResult::Success),
            _ => None,
        }
    }
    /// Whether `stack` stays in use until the button lets go (bows,
    /// crossbows, tridents, spyglasses, shields, eyes of ender, whose
    /// `getUseDuration` 0 never counts down to the end) or the server ends
    /// it (a mod's item): its use only counts ticks here.
    pub(in crate::game::client_side) fn held_until_released(&self, stack: &ItemStack) -> bool {
        if self.mod_use(stack) {
            return true;
        }
        matches!(
            &*stack.item,
            "minecraft:bow"
                | "minecraft:crossbow"
                | "minecraft:trident"
                | "minecraft:spyglass"
                | "minecraft:ender_eye"
        ) || sunrise_sim::spear_sim::Kinetic::of(&stack.item).is_some()
            || self.blocks_attacks(stack).is_some()
            || self.patched_held_use(stack)
    }
    /// `Hud.onDisconnected`, from `Minecraft.disconnect` and `clearClientLevel`
    /// (a proxy's reconfiguration too), but the chat: joining clears it and a
    /// reconfiguration keeps it (`ChatComponent.storeState`).
    pub(in crate::game::client_side) fn hud_disconnected(&mut self) {
        self.client.tab_list.0.reset();
        self.client.boss_overlay.reset();
        self.client.toast_manager = Default::default();
        self.client.debug_charts.reset();
        self.client.titles.clear();
        self.client.titles.reset_times();
    }
    /// `ServerPlayer.dismountTo` lands a rider at the ghast's own (x, maxY, z)
    /// (`HappyGhast.getDismountLocationForPassenger`). A vanilla client
    /// simulates the ghast it rides, so its copy is where the server has it;
    /// this window's copy trails the server's by the stepped interpolation and
    /// would rise into the rider's feet, which lets the rider fall through.
    /// Put the copy under the landing spot.
    pub(in crate::game::client_side) fn place_ghast_under(&mut self, id: u32, spot: Vec3) {
        let Some(m) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && m.species() == sunrise_sim::mobs::Species::HappyGhast)
        else {
            return;
        };
        let at = Vec3::new(spot.x, spot.y - f64::from(m.height()), spot.z);
        // Only a teleport that is that dismount: right above the ghast.
        if (at.x - m.position.x).powi(2) + (at.z - m.position.z).powi(2) > 25.0
            || (at.y - m.position.y).abs() > 5.0
        {
            return;
        }
        m.position = at;
        m.previous = at;
        if let Some(handler) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.stepped.get_mut(&(id as i32)))
        {
            handler.cancel();
        }
    }
    /// MultiPlayerGameMode.useItemOn inside startPrediction, for a block item:
    /// the blocks `BlockItem.place` set at `pos` are placed on this client's
    /// level at once, so the next click of a fast bridge aims at them rather
    /// than past them (the server would refuse it). Its place sound is the
    /// placer's own (`playSound(player, ...)`: the server sends it to the
    /// others only).
    pub fn predict_place(&mut self, placed: &DryPlace, stack: &ItemStack, seq: i32) {
        let pos = placed.pos;
        let at = [pos.x, pos.y, pos.z];
        // The dry simulation puts the stack state in the primary write. The
        // live copy takes item components before replaying the other cells.
        let Some((primary, rest)) = placed.changes.split_first() else {
            return;
        };
        self.predict_blocks(std::slice::from_ref(primary), seq);
        if placed.matches_candidate {
            self.apply_predicted_item(at, stack);
        }
        self.predict_blocks(rest, seq);
        let state = placed.sound_state;
        if !self.common.res.facts().is_air(state) {
            let sound = self
                .own_block_sound(at, state, mod_api::BlockSoundCause::Place)
                .for_placement(&stack.item);
            self.play_client_local_sound(LevelSound {
                name: sound.event.as_str(),
                at: at.map(|n| f64::from(n) + 0.5),
                volume: sound.volume,
                pitch: sound.pitch,
                source: mod_api::SoundSource::Blocks,
                emitter: None,
            });
        }
    }
    /// `handleAnimate`: a player wakes up, or an entity gets the critical
    /// hit (`CRIT`) or magic critical hit (`ENCHANTED_HIT`) emitter.
    pub(in crate::game::client_side) fn remote_animate(&mut self, id: i32, action: u8) {
        use sunrise_level::particle_options::{ParticleOptions, ParticleType};
        let particle = match action {
            // Player.stopSleepInBed.
            0 => {
                if self.common.player.is_some()
                    && self
                        .client
                        .remote
                        .as_ref()
                        .is_some_and(|remote| remote.entity_id == id)
                {
                    self.common.sleeping = None;
                    self.common.sleep_timer = 100;
                }
                return;
            }
            1 => ParticleType::Crit,
            2 => ParticleType::EnchantedHit,
            _ => return,
        };
        let Some((target, body)) = self.remote_tracking_body(id) else {
            return;
        };
        self.client.particles.0.tracking_emitter(
            ParticleOptions::simple(particle),
            target,
            body,
            None,
        );
    }
    /// Run BlockItem.place on a disposable level for a click: where it placed
    /// and the blocks it set. The real level supplies collision shapes and
    /// building-blocking entities; random draws, scheduled ticks and events
    /// stay in the preview. `ItemStack.useOn` runs it in adventure mode only
    /// when the used stack's `can_place_on` names the clicked block.
    pub fn remote_can_place(
        &mut self,
        hit: &super::super::Hit,
        stack: &ItemStack,
    ) -> Option<DryPlace> {
        self.remote_place_preview(hit, stack, true)
    }
    pub(in crate::game) fn remote_continue_attack_inner(&mut self, down: bool) {
        if !down {
            self.common.clicks.miss_time = 0;
        }
        // A piercing weapon neither breaks blocks nor swings on its own.
        if self.common.clicks.miss_time > 0
            || self.common.using.is_some()
            || self.client.client_item_use.0.is_some()
            || self
                .common
                .held()
                .is_some_and(|s| sunrise_sim::spear_sim::Kinetic::of(&s.item).is_some())
        {
            return;
        }
        let entity = self.entity_targeted();
        if down
            && !entity
            && let Some(hit) = self.pick()
        {
            let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let air = self.common.res.facts().is_air(state);
            // InputEvent.InteractionKeyMappingTriggered, before the dig goes on:
            // a cancel digs nothing and swings unless the mod said not to.
            let canceled = if air {
                None
            } else {
                self.native_interaction(
                    InteractionKey::Attack,
                    Hand::Main,
                    mod_api::InteractionAttempt::Held,
                )
                .canceled_swing()
            };
            if canceled.is_none() && !air {
                // MultiPlayerGameMode.continueDestroyBlock starts with ensureHasSentCarriedItem.
                self.remote_send_carried();
            }
            if let Some(swing) = canceled {
                if swing {
                    self.remote_swing_packet();
                }
            } else if self.remote_break_restricted(hit.pos) {
                // continueDestroyBlock returned false: no swing.
            } else if !air {
                if self.remote_mining(false) {
                    self.remote_swing_packet();
                }
            } else {
                if let Some(r) = self.client.remote.as_mut() {
                    r.destroying = None;
                    r.is_destroying = false;
                }
                self.set_client_mining(None);
            }
        } else {
            // `stopDestroyBlock` returns at once for a break that is not running: a pick that
            // missed once a break stopped ends nothing, so the dig's accumulated progress stays
            // and goes on growing on the jumps that follow (#4124).
            if self.stop_remote_mining() {
                self.set_client_mining(None);
            }
        }
    }
    pub fn remote_handle_input(&mut self) {
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let stop = std::mem::take(&mut r.stop_mining);
        let ready =
            r.loaded_sent && self.common.sleeping.is_none() && !self.mod_mappings_screen_open();
        self.side_refresh_client_pick(1.0);
        if stop || !ready {
            self.stop_remote_mining();
        }
        if ready {
            sunrise_client::interaction_input::tick(&mut RemoteInputWorld(self));
        }
        self.finish_agent_attack_input(ready);
        if ready && self.common.clicks.miss_time > 0 {
            self.common.clicks.miss_time -= 1;
        }
    }
    /// The server's death message; chat uses the shared ChatComponent.
    pub(in crate::game::client_side) fn remote_hud(&self, batch: &mut HudBatch) {
        let Some(r) = &self.client.remote else { return };
        let atlas = &self.client.gui.atlas;
        let w = batch.width;
        if self.client.screen == Screen::Dead
            && let Some(message) = &r.death_message
        {
            // DeathScreen draws the styled cause component.
            let tw = atlas.font.width_text(message);
            batch.text_styled(
                atlas,
                message,
                ((w - tw) / 2.0).floor(),
                85.0,
                u32::MAX,
                true,
            );
        }
    }
    /// `ClientPacketListener.handleLookAt`: `Entity.lookAt` from this
    /// player's feet or eyes, towards the packet's entity (its feet or eyes)
    /// when this client tracks it, else towards the packet's point.
    pub(in crate::game) fn remote_look_at(&mut self, p: &play::PlayerLookAt) {
        // `EntityAnchorArgument.Anchor`: 0 feet, 1 eyes.
        const EYES: i32 = 1;
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        let mut from = [player.position.x, player.position.y, player.position.z];
        if p.from == EYES {
            from[1] += f64::from(player.dimensions.eye_height);
        }
        let to = p
            .entity
            .as_ref()
            .and_then(|e| {
                let m = self.common.mobs.iter().find(|m| m.id == e.id as u32)?;
                let y = if e.anchor == EYES {
                    m.eye_y()
                } else {
                    m.position.y
                };
                Some([m.position.x, y, m.position.z])
            })
            .unwrap_or(p.target);
        (self.common.yaw, self.common.pitch) = look_at_rotation(from, to);
    }
    pub(in crate::game) fn remote_start_attack_inner(&mut self) -> bool {
        if self.common.clicks.miss_time > 0 || self.hands_busy() {
            return false;
        }
        if self.common.spectator() {
            // Minecraft.startAttack: a spectator's click on an entity asks
            // to watch through it (MultiPlayerGameMode.spectate); a click on
            // nothing still tells the server (spectatorNoAction).
            let target = self.spectator_click_target();
            self.send(S::SpectatorAction(play::SpectatorAction { target }));
            return true;
        }
        let spear = self
            .common
            .held()
            .and_then(|s| sunrise_sim::spear_sim::Kinetic::of(&s.item));
        if spear.is_some() && self.cannot_attack_with_item(self.client.hand_strength.ticker, 0) {
            return false;
        }
        // InputEvent.InteractionKeyMappingTriggered, after the checks that stop
        // an attack: a cancel ends it there, swinging unless the mod said not to.
        if let Some(swing) = self
            .native_interaction(
                InteractionKey::Attack,
                Hand::Main,
                mod_api::InteractionAttempt::Press,
            )
            .canceled_swing()
        {
            if swing {
                let animation = spear.map_or(SwingAnimation::DEFAULT, |k| k.attack_animation());
                self.swing_hand(Hand::Main, animation);
            }
            return false;
        }
        // A spear jabs (`MultiPlayerGameMode.piercingAttack`): this client
        // reports the reach and combo of a mob in range, then the server stabs
        // along the look it last heard (the jab sends none of its own) while
        // this client swings, plays the sound and starts its charge again.
        if let Some(kinetic) = spear {
            if let Some((i, distance, _)) = self.mob_hit()
                && self.before_block(distance)
                && self.held_range_allows(distance)
            {
                self.pvp_attacked(self.common.mobs[i].id as i32, distance);
            }
            // ServerboundPlayerActionPacket.Action.STAB.
            self.send(S::PlayerAction(play::PlayerAction {
                action: 8,
                pos: [0; 3],
                direction: 0,
                sequence: 0,
            }));
            self.client.hand_strength.reset_attack();
            self.swing_hand(Hand::Main, kinetic.attack_animation());
            self.client_play_at_feet(kinetic.attack_sound);
            return true;
        }
        if self.mod_attack_targets() {
            self.remote_swing_packet();
            return false;
        }
        let mut instant = false;
        if let Some(hit) = self.crosshair_entity_hit() {
            // A range rejection is still an entity hit: skip the attack, but swing below.
            if let Some(distance) = self.common.attack_hit_distance(hit.point) {
                self.remote_attack_entity(hit.id, distance);
            }
        } else if let Some(hit) = self.pick() {
            // A block hit on air falls through to MISS. `instant` below reads the
            // block again after the dig, which may have broken it.
            if mod_api::__private::listened(mod_api::HookId::LeftClickEmpty)
                && self
                    .common
                    .res
                    .facts()
                    .is_air(self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
            {
                self.native_left_click_empty();
            }
            if !self.remote_mining(true) {
                return false;
            }
            instant = self
                .common
                .res
                .facts()
                .is_air(self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        } else {
            if mod_api::__private::listened(mod_api::HookId::LeftClickEmpty) {
                self.native_left_click_empty();
            }
            if self.common.has_miss_time() {
                self.common.clicks.miss_time = 10;
            }
            self.reset_attack_strength();
        }
        self.remote_swing_packet();
        instant
    }
    /// Minecraft.startUseItem: use one crosshair target, then the item itself.
    pub(in crate::game) fn remote_use_inner(&mut self, attempt: mod_api::InteractionAttempt) {
        // Another queued press in the same input drain cannot restart a capture.
        if self.client.client_item_use.0.is_some() {
            return;
        }
        // Minecraft.startUseItem returns before resetting rightClickDelay
        // while a survival block break is active.
        if self.client.remote.as_ref().is_some_and(|r| r.is_destroying) {
            return;
        }
        self.common.use_delay = 4;
        // Minecraft.startUseItem: nothing is used while the hands paddle a boat.
        if self.hands_busy() {
            return;
        }
        // InputEvent.InteractionKeyMappingTriggered for the main hand: a cancel
        // ends the whole click, the off hand gets no turn.
        if let Some(swing) = self
            .native_interaction(InteractionKey::Use, Hand::Main, attempt)
            .canceled_swing()
        {
            if swing {
                self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
            }
            return;
        }
        // A registered client item behavior consumes the click before target handling.
        if self.start_client_item_use() {
            return;
        }
        // Resolve the crosshair once. A passing entity interaction keeps
        // that target and may only fall back to using the item itself.
        let entity = self.remote_use_targets_entity();
        sunrise_client::interaction_input::use_on_target(
            &mut RemoteUseWorld(self, attempt),
            entity,
        );
    }
    /// Whether a click would place `stack` (the controller guide's look): as
    /// `remote_can_place`, by the templates' rules alone, so looking asks no
    /// mod's placement or survival hook.
    pub fn remote_would_place(&mut self, hit: &super::super::Hit, stack: &ItemStack) -> bool {
        self.remote_place_preview(hit, stack, false).is_some()
    }
    /// Whether `held`'s `Item.use` needs a target this client sees none of
    /// (not SUCCESS): BucketItem.use (a fluid source or powder snow to pick
    /// up through `ClipContext.Fluid.SOURCE_ONLY`, a block to empty a full
    /// one against, `mayUseItemAt`), BottleItem.use (water to fill from) and
    /// BoatItem.use (a block or fluid surface with room for the boat).
    pub(in crate::game::client_side) fn use_finds_no_target(&self, held: &ItemStack) -> bool {
        let id = &*held.item;
        let facts = |pos: [i32; 3]| {
            self.common
                .res
                .facts()
                .get(self.common.world.block(pos[0], pos[1], pos[2]))
        };
        let source = |want: Option<Fluid>| {
            let fluid_hit = self.pick_fluid();
            // pick_fluid steps through waterlogged blocks, but their water is
            // a source the SOURCE_ONLY ray stops at (and a BucketPickup).
            if let Some(hit) = self.pick()
                && facts(hit.pos).waterlogged
            {
                let eye = self.aim().map_or(hit.point, |(eye, _)| eye);
                let d = |p: [f64; 3]| (0..3).map(|a| (p[a] - eye[a]).powi(2)).sum::<f64>();
                if fluid_hit
                    .as_ref()
                    .is_none_or(|fluid| d(fluid.point) >= d(hit.point))
                {
                    return want.is_none_or(|want| want == Fluid::Water);
                }
            }
            fluid_hit.is_some_and(|hit| {
                let f = facts(hit.pos);
                let fluid = f.fluid != Fluid::None && f.fluid_level == 8 && !f.fluid_falling;
                match want {
                    Some(want) => fluid && f.fluid == want,
                    None => fluid || self.common.is_powder_snow(hit.pos),
                }
            })
        };
        // SolidBucketItem only has useOn: its use is Item.use's PASS.
        if id == "minecraft:powder_snow_bucket" {
            return true;
        }
        if id == "minecraft:bucket" {
            return !self.may_build() || !source(None);
        }
        if sunrise_items::items::bucket_contents(id).is_some() {
            return !self.may_build() || self.pick().is_none();
        }
        if id == "minecraft:glass_bottle" {
            // A dragon's breath cloud never counts here: its owner is not
            // synced, so this client's BottleItem.use finds none and PASSes;
            // the packet still goes and the server fills the bottle.
            return !source(Some(Fluid::Water));
        }
        if sunrise_gameplay_common::game::boats_common::any_boat_wood(id).is_some() {
            return self.boat_to_place(held).is_none();
        }
        false
    }
    /// Runs one interaction and restores the caller's swing policy afterwards, on every return path.
    pub(in crate::game) fn with_interaction_swing_scope<R>(
        &mut self,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous = self
            .client
            .remote
            .as_ref()
            .map(|remote| remote.interaction_swing);
        let result = action(self);
        if let (Some(remote), Some(previous)) = (self.client.remote.as_mut(), previous) {
            remote.interaction_swing = previous;
        }
        result
    }

    pub fn remote_use(&mut self, attempt: mod_api::InteractionAttempt) {
        self.with_interaction_swing_scope(|game| game.remote_use_inner(attempt));
    }
    pub fn remote_start_attack(&mut self) -> bool {
        // An agent's `attack ID` checks its entity against the pick its click is taken with,
        // and its click on another target is dropped before it acts.
        if self.agent_attack_click_refused() {
            return false;
        }
        self.with_interaction_swing_scope(|game| game.remote_start_attack_inner())
    }
    pub fn remote_continue_attack(&mut self, down: bool) {
        self.with_interaction_swing_scope(|game| game.remote_continue_attack_inner(down));
    }
    /// `MultiPlayerGameMode.releaseUsingItem`: sends `RELEASE_USE_ITEM` and
    /// runs the client half of `Item.releaseUsing` (a Riptide launch moves the
    /// player itself). Vanilla reaches it both when the use key is let go and,
    /// in the same tick, when a keybind opens a screen.
    pub fn release_remote_use(&mut self) {
        self.send_action(6);
        // a mod's Stop may skip the item's release; a mod's release, then on_stop_using, follow.
        let hand = self.common.used_hand();
        if let Some((slot, ticks)) = self.common.using.take()
            && let Some(hand) = hand
            && let Some(stack) = self.common.stack_in(slot).cloned()
        {
            self.with_hand(hand, |g| {
                if g.use_item_stopped(&stack, ticks) {
                    if &*stack.item == "minecraft:trident" {
                        let held_ticks = g.ticks_held(&stack, ticks);
                        g.client_release_trident(held_ticks);
                    }
                    g.mod_release_using(&stack, ticks);
                }
                g.stopped_using(
                    &stack,
                    Some(g.own_entity_id()),
                    hand,
                    sunrise_gameplay_common::game::item_use_common::ticks_left(
                        g.common.use_duration,
                        ticks,
                    ),
                );
            });
        }
    }
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

use sunrise_gameplay_common::game::spear::SpearGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
