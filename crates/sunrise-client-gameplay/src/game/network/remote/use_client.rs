use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn finish_remote_use_block(
        &mut self,
        hand: Hand,
        before: Option<u32>,
        swings: u32,
        done: bool,
    ) {
        if done
            && self.common.swings_started != swings
            && let Some(after) = self.with_hand(hand, |g| g.held().map(|s| s.count))
            && (Some(after) != before || self.common.creative)
        {
            self.client.heights.item_used(hand);
        }
    }
    /// Whether the block hit takes a main-hand right click with `held`
    /// (`BlockState.useItemOn`, then `useWithoutItem`), once the caller let
    /// the block react (no sneaking with full hands): a door, a button, a
    /// container, and the blocks it changed. Vanilla's client runs it on its
    /// own level first; this runs it on a copy, and `predict_use` keeps the
    /// changes it predicts. `asks_mods` as for `DryLevel`.
    pub fn remote_block_reacts(
        &mut self,
        hit: &sunrise_gameplay_common::game::Hit,
        held: Option<&ItemStack>,
        asks_mods: bool,
    ) -> (InteractionResult, DryUse) {
        if let Some(reacts) = self.remote_block_shortcut(hit, held) {
            return reacts;
        }
        let [x, y, z] = hit.pos;
        let state = self.common.world.block(x, y, z);
        let (result, changes) = self.remote_block_template(hit, state, held, None, asks_mods);
        (result.unwrap_or(InteractionResult::Pass), changes)
    }
    /// Main-hand client `useItemOn` branches handled outside the dry level.
    pub(in crate::game) fn remote_block_shortcut(
        &mut self,
        hit: &sunrise_gameplay_common::game::Hit,
        held: Option<&ItemStack>,
    ) -> Option<(InteractionResult, DryUse)> {
        // AbstractCauldronBlock.useItemOn (CauldronInteractions): a bucket,
        // bottle or dye the cauldron takes fills or lowers it, and the click
        // ends there instead of falling through to Item.use (which would
        // empty a water bucket into the face of the cauldron).
        if let Some(reacts) = self.remote_cauldron_reacts(hit, held) {
            return Some(reacts);
        }
        if self.common.jukebox_takes(hit.pos, held) {
            return Some((InteractionResult::Success, DryUse::default()));
        }
        // RespawnAnchorBlock.useItemOn: PASS when the off hand's glowstone is to
        // charge the anchor, so useWithoutItem doesn't run and the off hand's turn
        // comes (glowstone in this hand charges it in `use_item_on`).
        if !self.common.anchor_takes(hit.pos, held)
            && self
                .common
                .anchor_takes(hit.pos, self.common.offhand.as_ref())
        {
            return Some((InteractionResult::Pass, DryUse::default()));
        }
        // BeehiveBlock.useItemOn: a glass bottle on a full hive (honey level 5)
        // is SUCCESS, so the click ends after UseItemOn and no UseItem follows.
        // Shears harvest only on a server level (`use_beehive`): on the client
        // they fall through to the empty-hand use, and the click goes on.
        if held.is_some_and(|s| &*s.item == "minecraft:glass_bottle")
            && self
                .common
                .hive_state(hit.pos)
                .is_some_and(|(honey, _)| honey == Some(5))
        {
            return Some((InteractionResult::Success, DryUse::default()));
        }
        // ComposterBlock.useItemOn: a compostable item on a composter below
        // level 8 is SUCCESS on the client too (only the server adds the
        // layer), so the other hand (a torch) is never tried.
        if self.composter_takes(hit.pos, held) {
            return Some((InteractionResult::Success, DryUse::default()));
        }
        self.client_block_answer(hit, held)
            .map(|result| (result, DryUse::default()))
    }
    pub fn remote_block_template(
        &mut self,
        hit: &sunrise_gameplay_common::game::Hit,
        state: BlockStateId,
        held: Option<&ItemStack>,
        branch: Option<sunrise_world::level::UseBranch>,
        asks_mods: bool,
    ) -> (Option<InteractionResult>, DryUse) {
        use sunrise_core::geometry::Direction;
        use sunrise_world::level::UseBranch;
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        let player = sunrise_world::level::PlayerContext {
            yaw: self.common.yaw,
            pitch: self.common.pitch,
            may_build: self.may_build(),
            can_eat: self.common.creative
                || self
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.health.food.food < 20),
            game_master: self.common.can_use_game_master_blocks(),
            respawn_point: None,
        };
        self.with_block_use_level(|b, level, live| {
            let mut level = DryLevel::new(level);
            level.reads = Some(live);
            level.asks_mods = asks_mods;
            let hit = sunrise_core::geometry::BlockHit {
                block: sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                direction: face,
                location: sunrise_core::geometry::Vec3::new(
                    hit.point[0],
                    hit.point[1],
                    hit.point[2],
                ),
                inside: hit.inside,
            };
            let mut ticks = sunrise_world::level::Ticks::default();
            let item = held.map(|s| &*s.item);
            let result = match branch {
                Some(UseBranch::ItemOn) => {
                    b.block_use_item_on(&mut level, &mut ticks, state, hit, player, item)
                }
                Some(UseBranch::WithoutItem) => {
                    Some(b.use_without_item(&mut level, &mut ticks, state, hit, player))
                }
                None => Some(
                    b.use_item_on_branch(&mut level, &mut ticks, state, hit, player, item)
                        .0,
                ),
            };
            (result, (level.changed.into_iter().collect(), level.sounds))
        })
    }
    /// A container click: the server answers with the authoritative slots.
    pub(in crate::game::client_side) fn remote_click(&mut self, right: bool) -> bool {
        let Some(size) = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let Some(internal) = self.slot_at(&batch, self.client.cursor.0, self.client.cursor.1)
        else {
            return false;
        };
        let input = if self.has_shift_down() {
            Input::QuickMove
        } else {
            Input::Pickup
        };
        if self.client.screen == Screen::Creative {
            // CreativeModeInventoryScreen.slotClicked: the Survival Inventory tab clicks the
            // player's own InventoryMenu, and its CreativeInventoryListener sends each slot that
            // changed (SetCreativeModeSlot). No ContainerClick: a server holding the carried
            // stack would give it back after the Destroy Item slot cleared it (#3307).
            self.click_open_inventory(Some(internal), i32::from(right), input);
            return true;
        }
        self.remote_menu_ops(&[(Some(internal), i32::from(right), input)]);
        true
    }
    /// `Player.handleEntityEvent(9)`: `completeUsingItem` on the client. A consumable's last crumbs, and FoodProperties.onConsume,
    /// which runs on the client too (Consumable.onConsume calls its
    /// ConsumableListeners on both sides): the client's FoodData eats at once,
    /// so a held right click finds `Player.canEat` false instead of starting
    /// another bite before the server's ClientboundSetHealth arrives.
    pub fn remote_complete_using(&mut self) {
        let Some((slot, _)) = self.common.using.take() else {
            return;
        };
        let Some(stack) = self.common.stack_in(slot).cloned() else {
            return;
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return;
        };
        // Consumable.onConsume's emitParticlesAndSounds(16): the last crumbs
        // and bite, which the server plays for the others only.
        if let Some(consumable) = self.consumable(&stack) {
            self.emit_consume(&consumable, &stack, 16);
        }
        if let Some((nutrition, saturation)) = info.food
            && let Some(r) = self.client.remote.as_mut()
        {
            // FoodData.eat(nutrition, saturation), as finish_consuming does;
            // the next ClientboundSetHealth replaces it.
            let (_, food, sat) = &mut r.health;
            *food = (*food + nutrition as i32).clamp(0, 20);
            *sat = (*sat + saturation).clamp(0.0, *food as f32);
            self.apply_remote_health();
        }
        // applyAfterUseComponentSideEffects, then LivingEntityUseItemEvent.Finish.
        self.remote_use_item_finished(slot, &stack, &info);
    }
    pub(in crate::game::client_side) fn remote_place_preview(
        &mut self,
        hit: &sunrise_gameplay_common::game::Hit,
        stack: &ItemStack,
        asks_mods: bool,
    ) -> Option<DryPlace> {
        if !self.may_build() && !self.stack_can_place_on_in_adventure(stack, hit.pos) {
            return None;
        }
        let ctx = sunrise_world::level::PlaceContext {
            clicked_pos: sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face: sunrise_core::geometry::Direction::ALL[face_direction(hit.face) as usize],
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: hit.inside,
            source: sunrise_world::level::PlacementSource::Player {
                yaw: self.common.yaw,
                pitch: self.common.pitch,
                sneaking: self.remote_shift_down(),
            },
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.common.can_use_game_master_blocks(),
        };
        let registry = self.common.pack.assets.registry;
        let stack_state = |state| {
            sunrise_gameplay_common::game::block_items::item_block_state(
                registry,
                state,
                stack,
                ctx.can_use_game_master_blocks,
            )
        };
        let has_part = asks_mods
            && mod_api::__private::item_listens(mod_api::HookId::PlacementContext, &stack.item);
        let placer = has_part.then(|| self.placement_placer()).flatten();
        self.with_level(|b, level, ticks| {
            let mut dry = DryLevel::new(level);
            dry.reads = Some(ticks);
            dry.asks_mods = asks_mods;
            let mut ticks = sunrise_world::level::Ticks::default();
            // The rewrite reads the base level: the scratch level has not written anything yet.
            let (decision, _) =
                b.decide_placement(&mut dry, &mut ticks, &ctx, stack_state, |_, context| {
                    has_part
                        .then(|| level.rewrite_placement_context(stack, context, placer))
                        .flatten()
                })?;
            let placed = b.apply_placement(&mut dry, &mut ticks, decision, stack_state)?;
            b.placement_set_placed_by(&mut dry, &mut ticks, &placed);
            let pos = placed.context.pos;
            let primary = dry.changed.remove_entry(&pos);
            Some(DryPlace {
                pos,
                sound_state: placed.state,
                matches_candidate: placed.matches_candidate,
                changes: primary.into_iter().chain(dry.changed).collect(),
            })
        })
    }
    /// `Consumable.onConsumeTick` for another player using a consumable: the
    /// crumbs at their mouth every 4 ticks once 21.875% of its use has passed
    /// (`shouldEmitParticlesAndSounds`; the sounds come from the server).
    pub(in crate::game::client_side) fn remote_player_eating(
        &mut self,
        item: &str,
        ticks: u32,
        at: [f64; 3],
        eye: f64,
        pitch: f32,
        yaw: f32,
    ) {
        let Some(consumable) = self
            .common
            .res
            .items()
            .get(item)
            .and_then(|i| i.consumable.as_ref())
            .filter(|c| c.particles)
        else {
            return;
        };
        let duration = consumable.ticks().max(0) as u32;
        let remaining = duration - ticks.min(duration);
        if !consumable.should_emit(remaining as i32) {
            return;
        }
        let options = self.item_particle(&ItemStack::new(item, 1));
        let mut spawned = Vec::new();
        sunrise_level::particle_options::item_use_particles(
            &mut self.common.world.random,
            pitch,
            yaw,
            at,
            at[1] + eye,
            5,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.client.particles.0.add(&options, p, v);
        }
    }
    /// `Item.use` runs on the client too (MultiPlayerGameMode.useItem): an
    /// item held in use until the button lets go starts being used here, so
    /// that letting go sends RELEASE_USE_ITEM (Minecraft.handleKeybinds) and
    /// the server ends the use: a bow or a trident is let go, a crossbow's
    /// charge ends, a shield is lowered, a spyglass stops scoping. Their
    /// sounds, loaded arrows and thrown tridents come from the server.
    /// False when the use FAILs here: a bow or an uncharged crossbow with
    /// nothing to shoot.
    pub(in crate::game::client_side) fn remote_start_held_use(&mut self, held: &ItemStack) -> bool {
        if self.common.using.is_some() {
            return true;
        }
        let bundle = self.items().get(&held.item).is_some_and(|i| i.bundle);
        let start = match &*held.item {
            // BundleItem.use: always starts; the server drops the contents.
            _ if bundle => true,
            // BowItem.use: with an arrow, or in creative.
            "minecraft:bow" => self.get_projectile(held).is_some() || self.common.creative,
            // CrossbowItem.use: charging when not charged and with ammunition;
            // a charged one shoots on the server.
            "minecraft:crossbow" => {
                if held.data().is_some_and(|d| !d.charged.is_empty()) {
                    return true;
                }
                self.get_projectile(held).is_some()
            }
            // TridentItem.use: not about to break, Riptide only in water or rain.
            "minecraft:trident" => {
                // FAIL: no dip, and the off hand gets its turn.
                return self.use_trident(held);
            }
            // SpyglassItem.use (its sound is the server's).
            "minecraft:spyglass" => true,
            // InstrumentItem.use: the horn plays here too (`playSound(player, ...)`
            // skips the player's own client), the server's copy tells the others.
            "minecraft:goat_horn" => {
                self.remote_use_instrument(held);
                return true;
            }
            // EnderEyeItem.use: startUsingItem unless an End portal frame is
            // aimed at, with or without a stronghold to throw the eye at.
            "minecraft:ender_eye" => !self.aims_at_end_portal_frame(),
            // Item.use: a kinetic weapon starts being used.
            id if sunrise_sim::spear_sim::Kinetic::of(id).is_some() => true,
            // ShieldItem.use: anything that blocks attacks, off cooldown.
            // Item.use: a consumable or `blocks_attacks` the server's patch
            // adds starts the use (Consumable.startConsuming, startUsingItem).
            _ => {
                let default = self
                    .items()
                    .get(&held.item)
                    .and_then(|i| i.use_cooldown.as_ref());
                let group = sunrise_items::cooldowns::group(held, default);
                if self.blocks_attacks(held).is_some() {
                    if !self.common.cooldowns.is_on_cooldown(&group) {
                        self.start_using_item();
                    }
                } else if self.patched_held_use(held) {
                    self.start_using_item();
                }
                return true;
            }
        };
        if start {
            self.start_using_item();
        }
        start || !matches!(&*held.item, "minecraft:bow" | "minecraft:crossbow")
    }
    /// `Minecraft.startUseItem`'s block turn for `hand`: a use that swung drops
    /// the hand (`LocalPlayer.itemUsed`) when it spent the held item or the
    /// player has infinite materials.
    pub fn remote_use_block(
        &mut self,
        hand: Hand,
        attempt: mod_api::InteractionAttempt,
        hit: Option<Hit>,
    ) -> bool {
        let before = self.with_hand(hand, |g| g.held().map(|s| s.count));
        let swings = self.common.swings_started;
        let done = self.remote_use_item_on(hand, attempt, hit);
        self.finish_remote_use_block(hand, before, swings, done);
        done
    }
    /// `MultiPlayerGameMode.useItem`'s local `ItemStack.use` for a bucket:
    /// `BucketItem.use` picks its own target and runs `pickupBlock` or
    /// `emptyContents` on this client's level inside `startPrediction`, so
    /// its success (which decides `itemUsed`) and the blocks it changes are
    /// the client's own. A fill into a waterloggable block changes nothing
    /// here: `SimpleWaterloggedBlock.placeLiquid` tests `isClientSide` and
    /// leaves the block update to the server.
    pub(in crate::game::client_side) fn remote_use_bucket(
        &mut self,
        stack: &ItemStack,
    ) -> (bool, Option<DryChanges>, Vec<DrySound>) {
        // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
        // `Player.mayUseItemAt` tests the clicked block against `can_place_on`.
        if !self.may_build() {
            let aim = if &*stack.item == "minecraft:bucket" {
                self.bucket_hit()
            } else {
                self.pick()
            };
            if !aim.is_some_and(|hit| self.can_place_on_in_adventure(hit.pos)) {
                return (false, None, Vec::new());
            }
        }
        if &*stack.item == "minecraft:bucket" {
            let Some(hit) = self.bucket_hit() else {
                return (false, None, Vec::new());
            };
            let pos = sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.common.creative;
            return self.with_level(|b, level, ticks| {
                let mut level = DryLevel::new(level);
                let picked = b.bucket_pickup(&mut level, ticks, pos, creative).is_some();
                (
                    picked,
                    Some(level.changed.into_iter().collect()),
                    std::mem::take(&mut level.sounds),
                )
            });
        }
        let Some((content, sound, _)) = self.predicted_bucket_contents(stack) else {
            return (false, None, Vec::new());
        };
        let Some(hit) = self.pick() else {
            return (false, None, Vec::new());
        };
        let target = self.bucket_target(content, &hit);
        let pos = sunrise_core::position::BlockPos::new(target[0], target[1], target[2]);
        let [x, y, z] = hit.beside();
        let beside = Some(sunrise_core::position::BlockPos::new(x, y, z));
        let sneaking = self.remote_shift_down();
        let user = self.bucket_liquid_user();
        self.with_level(|b, level, ticks| {
            let mut level = DryLevel::new(level);
            let emptied = b.bucket_empty_as(
                &mut level, ticks, pos, beside, content, sound, sneaking, user,
            );
            (
                emptied.is_some(),
                (!level.filled_container).then(|| level.changed.into_iter().collect()),
                std::mem::take(&mut level.sounds),
            )
        })
    }
    /// Continue Minecraft.startUseItem after this hand's target passed:
    /// its item, then the off hand's target and item if this is the main hand.
    /// When resumed after a server answer, use the original block hit.
    pub fn remote_use_hands(
        &mut self,
        hand: Hand,
        attempt: mod_api::InteractionAttempt,
        block_hit: Option<Hit>,
    ) {
        // Nothing in the crosshair: an empty hand's click is heard, by mods only.
        let miss = block_hit.is_none()
            && mod_api::__private::listened(mod_api::HookId::RightClickEmpty)
            && !self.entity_targeted()
            && self.pick().is_none();
        if miss && self.with_hand(hand, |g| g.held().is_none()) {
            self.native_right_click_empty(hand);
        }
        // A successful Item.use always drops the hand (LocalPlayer.itemUsed).
        if self.remote_use_item(hand) {
            self.client.heights.item_used(hand);
            return;
        }
        if hand == Hand::Off {
            return;
        }
        // The off hand's turn: the event again, then the entity or the block.
        if let Some(swing) = self
            .native_interaction(InteractionKey::Use, Hand::Off, attempt)
            .canceled_swing()
        {
            if swing {
                self.swing_hand(Hand::Off, SwingAnimation::DEFAULT);
            }
            return;
        }
        let off_target = if block_hit.is_none() && self.remote_use_targets_entity() {
            self.remote_use_entity(Hand::Off)
        } else {
            self.remote_use_block(Hand::Off, attempt, block_hit)
        };
        if off_target {
            return;
        }
        if miss && self.common.offhand.is_none() {
            self.native_right_click_empty(Hand::Off);
        }
        if self.remote_use_item(Hand::Off) {
            self.client.heights.item_used(Hand::Off);
        }
    }
    /// `InstrumentItem.use` on this client: unless the horn is on cooldown,
    /// it is held up, plays its sound at the player and starts its cooldown.
    pub(in crate::game::client_side) fn remote_use_instrument(&mut self, held: &ItemStack) {
        let Some(instrument) = self.items().instrument(held) else {
            return;
        };
        let group = sunrise_items::cooldowns::group(held, None);
        if self.common.cooldowns.is_on_cooldown(&group) {
            return;
        }
        self.start_using_item();
        self.play_player_bound_sound(&instrument.sound, 2, instrument.range / 16.0, 1.0, true);
        if instrument.use_duration > 0.0 {
            self.common.cooldowns.add(group, instrument.use_ticks());
        }
    }

    /// `MultiPlayerGameMode.destroyDelay`: the ticks left before the local player's next block
    /// starts breaking. 0 outside a world.
    pub(in crate::game) fn mining_destroy_delay(&self) -> u32 {
        self.client.remote.as_ref().map_or(0, |r| r.destroy_delay)
    }

    /// MultiPlayerGameMode.startDestroyBlock / continueDestroyBlock.
    pub fn remote_mining(&mut self, start: bool) -> bool {
        let debug_stick = self
            .common
            .held()
            .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
        if !self
            .common
            .res
            .items()
            .can_destroy_block(self.common.held(), self.common.creative)
        {
            self.stop_remote_mining();
            self.set_client_mining(None);
            return true;
        }
        let Some(r) = self.client.remote.as_mut() else {
            return true;
        };
        if !start && r.destroy_delay > 0 {
            r.destroy_delay -= 1;
            return true;
        }
        let Some(hit) = self.pick() else {
            self.set_client_mining(None);
            return true;
        };
        let face = face_direction(hit.face);
        let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.common.res.facts().is_air(state) {
            return true;
        }
        // startDestroyBlock refuses a restricted break before predicting it.
        if self.remote_break_restricted(hit.pos) {
            self.set_client_mining(None);
            return true;
        }
        if self.common.creative {
            if self.veto_mining_prediction(hit.pos, hit.face, true, 1.0) {
                return false;
            }
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock never lets the client break the
            // block; the server only selects the property on this START.
            if !debug_stick {
                self.predict_break(hit.pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.client.remote.as_mut() {
                r.destroy_delay = 5;
            }
            return true;
        }
        // sameDestroyTarget: the same block and still the item the break began with.
        let same = self.client.remote.as_ref().is_some_and(|r| {
            r.destroying.is_some_and(|(p, _, _)| p == hit.pos)
                && self.same_destroying_item(&r.destroying_item)
        });
        // startDestroyBlock leaves only a break that is still running alone (`isDestroying &&
        // sameDestroyTarget`); a click on the block a stopped break still names starts it again,
        // so the server re-anchors its clock. A held button carries the same target on, and that
        // one keeps the clock (#4124).
        let running = self.client.remote.as_ref().is_some_and(|r| r.is_destroying);
        if start && same && running {
            return true;
        }
        // The server's own getDestroyProgress: under water, off the ground,
        // with haste, fatigue and efficiency, so the client stops digging
        // when the server is done too (#1254).
        let progress = self.destroy_progress(hit.pos);
        if !same || start {
            let proposed = if progress >= 1.0 { 1.0 } else { 0.0 };
            if self.veto_mining_prediction(hit.pos, hit.face, true, proposed) {
                return false;
            }
            // startDestroyBlock aborts only a break that is running; a target left by a stop that
            // ended one is just replaced by the START below. The abort leaves the running break's
            // state as it was: a break that isn't instant replaces it below, and after an instant
            // one the old target is still running, so the next start aborts it again.
            let aborted = self
                .client
                .remote
                .as_ref()
                .filter(|r| r.is_destroying)
                .and_then(|r| r.destroying);
            if let Some((old, _, _)) = aborted {
                let old_state = self.common.world.block(old[0], old[1], old[2]);
                self.tutorial_destroy(old_state, -1.0);
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 2,
                    pos: old,
                    direction: face,
                    sequence: 0,
                }));
            }
            let seq = self.next_sequence();
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            self.tutorial_destroy(state, 0.0);
            // startDestroyBlock: BlockState.attack on the client, so a redstone
            // ore's dust (RedStoneOreBlock.attack) comes before the break.
            self.redstone_ore_particles(hit.pos);
            if progress >= 1.0 {
                // startDestroyBlock: instant survival breaks have no destroy delay.
                if !debug_stick {
                    self.predict_break(hit.pos, state, seq);
                }
                return true;
            }
            let held = self.common.main_hand_key();
            if let Some(r) = self.client.remote.as_mut() {
                (r.destroying, r.destroying_item) = (Some((hit.pos, face, 0.0)), held);
                r.is_destroying = true;
            }
            // startDestroyBlock tells its level the new dig before the first hit's effects.
            self.native_local_break_progress(hit.pos, 0.0);
            // startDestroyBlock plays the first hit and crack itself; the
            // server's level events 2019 and 2020 do the rest.
            self.breaking_block_effects(hit.pos, hit.face, true);
            self.set_client_mining(Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                per_tick: 0.0,
                ticks: 0,
            }));
            return true;
        }
        let Some((pos, last_face, done)) = self.client.remote.as_ref().and_then(|r| r.destroying)
        else {
            return true;
        };
        let done = done + progress;
        if self.veto_mining_prediction(pos, hit.face, false, done) {
            return false;
        }
        self.tutorial_destroy(state, done.clamp(0.0, 1.0));
        let ticks = self.common.mining.as_ref().map_or(0, |m| m.ticks) + 1;
        if done >= 1.0 {
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock: the server selects the property
            // and sends the block update instead; never predict a break.
            if !debug_stick {
                self.predict_break(pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 3,
                pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.client.remote.as_mut() {
                // `destroyBlockPos` remains the last target after the break;
                // if that position is filled again before the next pick, a
                // held attack resumes it without sending a second START.
                r.destroying = Some((pos, face, 0.0));
                r.is_destroying = false;
                r.destroy_delay = 5;
            }
            self.set_client_mining(None);
            // continueDestroyBlock tells its level the reset progress: the crack goes.
            self.native_local_break_progress(pos, 0.0);
        } else {
            if last_face != face {
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 1,
                    pos,
                    direction: face,
                    sequence: 0,
                }));
            }
            if let Some(r) = self.client.remote.as_mut() {
                r.destroying = Some((pos, face, done));
            }
            self.set_client_mining(Some(Mining {
                pos,
                progress: done,
                per_tick: 0.0,
                ticks,
            }));
            self.native_local_break_progress(pos, done);
        }
        true
    }
}

use sunrise_gameplay_common::game::end_common::EndGameExt as _;

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::cake_eligibility::CakeEligibilityGameExt1 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::placement_context::PlacementContextGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
