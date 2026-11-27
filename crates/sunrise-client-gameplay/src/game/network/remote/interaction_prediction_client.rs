use super::*;
use sunrise_gameplay_common::game::item_abilities::{ItemAbilitiesGameExt1 as _, ability};
use sunrise_gameplay_common::game::powder_snow::FilledResultGameExt as _;

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn finish_remote_item_on(
        &mut self,
        hand: Hand,
        answer: mod_api::InteractionResult,
    ) -> bool {
        use mod_api::InteractionResult as Answer;
        if answer == Answer::Success {
            self.with_hand(hand, PresentationSideGameExt::swing_arm);
        }
        answer.consumes_action() || answer == Answer::Fail
    }
    /// `Item.useOn` on this client's level for the items whose effects are
    /// the clicker's own (`Level.levelEvent(player, ...)` and
    /// `playSound(player, ...)`: the server sends them to the others only):
    /// honeycomb waxing copper, and an axe, hoe or shovel transforming a
    /// block (stripping, scraping, removing wax, tilling, making a path).
    /// A cushion succeeds where it may be placed and spawns on the server, a
    /// firework rocket wherever it may be used, and the server launches it.
    /// Whether the item's use succeeded, which ends the click.
    /// The caller has selected Item.useOn after cooldown and adventure gates.
    pub fn predict_item_on_block(
        &mut self,
        stack: &ItemStack,
        hit: &sunrise_gameplay_common::game::Hit,
    ) -> bool {
        // FireworkRocketItem.useOn: SUCCESS unless gliding, when Item.use
        // boosts instead. The phase's cooldown/adventure gates already ran.
        if &*stack.item == "minecraft:firework_rocket" {
            return !self.common.player.as_ref().is_some_and(|p| p.fall_flying);
        }
        let at = [
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.5,
            f64::from(hit.pos[2]) + 0.5,
        ];
        if &*stack.item == "minecraft:honeycomb" {
            // HoneycombItem.useOn.
            let registry = self.common.pack.assets.registry;
            let clicked = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let old = registry.state(clicked);
            let waxes = old
                .and_then(|old| self.common.res.transformers().waxed(registry, old))
                .is_some();
            if waxes {
                self.client_level_event_at(3003, hit.pos, 0);
                self.play_client_local_sound(
                    LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                        .in_source(mod_api::SoundSource::Blocks),
                );
                // The other half of a double chest: its particles, and a second
                // sound from the clicked block.
                if let Some(other) =
                    old.and_then(|old| crate::game::storage::chest_connected_pos(old, hit.pos))
                {
                    self.client_level_event_at(3003, other, 0);
                    self.play_client_local_sound(
                        LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                            .in_source(mod_api::SoundSource::Blocks),
                    );
                }
            }
            waxes
        } else if let Some((_, sound, _, _, particle, _)) = self.block_transform(
            stack,
            hit,
            self.held_mapping(sunrise_client::actions::Action::Sneak),
        ) {
            match particle.as_str() {
                "scrape" => self.client_level_event_at(3005, hit.pos, 0),
                "wax_on" => self.client_level_event_at(3003, hit.pos, 0),
                "wax_off" => self.client_level_event_at(3004, hit.pos, 0),
                _ => {}
            }
            self.play_client_local_sound(
                LevelSound::new(&sound, at, 1.0, 1.0).in_source(mod_api::SoundSource::Blocks),
            );
            true
        } else if self
            .common
            .res
            .items()
            .get(&stack.item)
            .is_some_and(|i| i.cushion_color.is_some())
        {
            // CushionItem.useOn on a client level: SUCCESS once
            // `canBePlacedAt` holds, before the server's own checks.
            self.cushion_spawn(hit).is_some()
        } else if &*stack.item == "minecraft:painting" {
            // HangingEntityItem.useOn on a client level: SUCCESS once
            // `Painting.create` finds a surviving variant, before the
            // server spawns it.
            self.painting_fits(hit, stack)
        } else {
            false
        }
    }
    pub(in crate::game) fn receive_cauldron_use(&mut self, data: &[u8]) -> Result<()> {
        let mut body = sunrise_protocol::buf::Reader::new(data);
        let receipt = body.i64()?;
        let block_fell_through = body.bool()?;
        let passed = body.bool()?;
        if body.remaining() != 0 {
            return Err("extra bytes in authoritative block-use result".into());
        }
        let click = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.pending_cauldrons.get_mut().remove(&receipt));
        // Minecraft.startUseItem and Item.getPlayerPOVHitResult use the same
        // player and aim in one input turn. A UseItem packet can't supply an old
        // eye position, so changed input supersedes the suspended click.
        if (block_fell_through || passed)
            && let Some(click) = click
            && self.common.selected == click.selected
            && self.common.stack_in(click.selected) == click.main.as_ref()
            && self.common.offhand == click.off
            && self.remote_item_use_aim() == click.aim
        {
            self.with_interaction_swing_scope(|game| {
                if let Some(remote) = &mut game.client.remote {
                    remote.interaction_swing = click.swing;
                }
                if block_fell_through && click.item_after_block {
                    let held = match click.hand {
                        Hand::Main => click.main.as_ref(),
                        Hand::Off => click.off.as_ref(),
                    };
                    let swings = game.swings_started;
                    let answer = game.remote_item_after_block(&click.hit, click.hand, held);
                    if let Some(remote) = &mut game.client.remote {
                        remote.use_target = Some(click.hit.pos);
                    }
                    let done = if let Some(answer) = answer {
                        game.finish_remote_item_on(click.hand, answer)
                    } else {
                        let stand_fits = game.remote_armor_stand_fits(&click.hit, held);
                        game.remote_vanilla_item_on(
                            &click.hit,
                            click.hand,
                            held,
                            click.sequence,
                            stand_fits,
                        )
                    };
                    game.finish_remote_use_block(click.hand, held.map(|s| s.count), swings, done);
                    if done {
                        return;
                    }
                }
                if passed && game.remote_item_use_aim() == click.aim {
                    game.remote_use_hands(click.hand, click.attempt, Some(click.hit));
                }
            });
        }
        Ok(())
    }
    pub(in crate::game::client_side) fn remote_armor_stand_fits(
        &self,
        hit: &Hit,
        held: Option<&ItemStack>,
    ) -> bool {
        held.filter(|stack| &*stack.item == "minecraft:armor_stand")
            .is_some_and(|stack| self.common.armor_stand_spawn(hit, stack).is_some())
    }
    pub(in crate::game::client_side) fn remote_item_after_block(
        &mut self,
        hit: &Hit,
        hand: Hand,
        held: Option<&ItemStack>,
    ) -> Option<mod_api::InteractionResult> {
        self.use_item_on_block(hit, hand, held, mod_api::UsePhase::ItemAfterBlock)
            .or_else(|| held.and_then(|stack| self.item_on_interaction(hit, hand, stack)))
    }
    pub(in crate::game::client_side) fn remote_item_use_aim(&self) -> ItemUseAim {
        ItemUseAim {
            rotation: (self.common.yaw, self.common.pitch),
            eye: self.common.eye_position(),
            ray: self.aim(),
            reach: self.common.reach(),
        }
    }
    /// `Minecraft.startUseItem`'s entity turn for `hand`: Interact goes to
    /// the server, and whether the click ends there is what this client's
    /// copy of the entity answers (`MultiPlayerGameMode.interact`).
    pub fn remote_use_entity(&mut self, hand: Hand) -> bool {
        // A menu this use opens (a chest minecart's) is no clicked block's.
        if let Some(r) = self.client.remote.as_mut() {
            r.use_target = None;
        }
        let spectator = self.common.spectator();
        let sneaking = self.remote_shift_down();
        let interact_packet = |id, location| {
            S::Interact(play::Interact {
                id,
                hand: hand_id(hand),
                location,
                secondary: sneaking,
            })
        };
        if self.uses_stored_pick() || spectator {
            let Some(mod_api::CrosshairTarget::Entity { entity, point }) = self.crosshair_target()
            else {
                return false;
            };
            let Some(([lo, hi], origin)) = self.use_entity_geometry(entity) else {
                return false;
            };
            // Minecraft.startUseItem checks the target's blockPosition() against the world
            // border before range/interaction and returns from the whole click when outside.
            let block = sunrise_core::position::BlockPos::containing(Vec3::new(
                origin[0], origin[1], origin[2],
            ));
            if !self
                .common
                .border
                .contains(f64::from(block.x), f64::from(block.z))
            {
                return true;
            }
            let Some(eye) = self.common.eye_position() else {
                return false;
            };
            // Client interact range: closest box point, strict bound and no server buffer.
            // Refuse this use without choosing another target or the block behind it.
            let distance_squared = (0..3)
                .map(|i| (eye[i] - eye[i].clamp(lo[i], hi[i])).powi(2))
                .sum::<f64>();
            if distance_squared >= self.common.entity_reach.powi(2) {
                return false;
            }
            if spectator {
                // MultiPlayerGameMode.interact sends for either hand before returning PASS.
                let location = std::array::from_fn(|i| point[i] - origin[i]);
                self.send(interact_packet(entity.id() as i32, location));
                return false;
            }
        }
        // A boat, minecart, item frame, armor stand or painting nearer than
        // any mob: the server uses it (AbstractBoat.interact boards it).
        if let Some(hit) = self.remote_object_hit()
            && self.mob_hit().is_none_or(|(_, d, _)| hit.distance < d)
            && self.before_block(hit.distance)
        {
            // Interaction.interact answers SUCCESS when it responds and
            // CONSUME otherwise: both end the click (nothing behind the box
            // is used), but only SUCCESS swings; the packet carries the hand.
            let response = self.client.interactions.get(&hit.id).map(|i| i.response);
            // Objects whose interact passes (paintings, empty or fixed item frames,
            // boats sneaked at or out of control, carts that pass) leave each hand's
            // turn to its item: a crossbow aimed at it shoots.
            let is_painting = self.common.paintings.iter().any(|p| p.id as i32 == hit.id);
            let held = self.with_hand(hand, |g| g.held().cloned());
            let frame_passes = self
                .common
                .item_frames
                .iter()
                .find(|f| f.id as i32 == hit.id)
                .is_some_and(|f| {
                    item_frame_interact_passes(f.fixed, f.item.is_some(), held.is_some())
                });
            let boat_passes = self
                .common
                .boats
                .iter()
                .enumerate()
                .find(|(_, b)| b.id as i32 == hit.id)
                .is_some_and(|(i, b)| {
                    boat_interact_passes(
                        b.chest,
                        sneaking,
                        b.out_of_control >= 60.0,
                        self.boat_has_room(i),
                    )
                });
            let (cart_passes, is_rideable_cart) = self
                .common
                .carts
                .iter()
                .find(|c| c.id as i32 == hit.id)
                .map(|c| {
                    let ridden = self.common.riding == Some(c.id)
                        || self.common.mobs.iter().any(|m| m.extra.mount == Some(c.id));
                    (
                        c.kind.interact_passes(
                            sneaking,
                            ridden,
                            self.common.can_use_game_master_blocks(),
                        ),
                        c.kind == sunrise_gameplay_common::game::minecarts::CartKind::Rideable,
                    )
                })
                .unwrap_or((false, false));
            // Cushion.interact on a client: PASS when sneaked at or when
            // something already sits on it, CONSUME (no swing) otherwise.
            let is_cushion = self.common.cushions.iter().any(|c| c.id as i32 == hit.id);
            let cushion_passes = is_cushion
                && (sneaking
                    || self.common.riding.is_some_and(|r| r as i32 == hit.id)
                    || self
                        .client
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.has_passengers(hit.id as u32)));
            // EndCrystal, EnderDragonPart, PrimedTnt, and FallingBlock keep Entity.interact's PASS.
            let crystal_passes = self.common.crystals.iter().any(|c| c.id as i32 == hit.id)
                || self.common.dragon.as_ref().is_some_and(|d| {
                    sunrise_gameplay_common::game::dragon_common::dragon_part_index(
                        d.id,
                        hit.id as u32,
                    )
                    .is_some()
                });
            let tnt_or_falling_passes = self.common.tnts.iter().any(|t| t.id as i32 == hit.id)
                || self.common.falling.iter().any(|f| f.id as i32 == hit.id);
            let passes = self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.general.rows.contains_key(&hit.id))
                || is_painting
                || frame_passes
                || boat_passes
                || cart_passes
                || cushion_passes
                || crystal_passes
                || tnt_or_falling_passes;
            // Otherwise the main hand's turn takes it; there is no off hand turn.
            if response.is_none() && hand == Hand::Off && !passes {
                return false;
            }
            self.send(interact_packet(hit.id, hit.location));
            if let Some(done) = self.entity_interaction(hand, hit.id) {
                return done;
            }
            if passes {
                return false;
            }
            // Minecart.interact resets the passenger's easing baseline on
            // a successful boarding click, not on SetPassengers (commands
            // can mount a player without this interaction).
            if let Some(i) = self.common.carts.iter().position(|c| {
                c.id as i32 == hit.id
                    && c.kind == sunrise_gameplay_common::game::minecarts::CartKind::Rideable
            }) {
                self.common
                    .carts
                    .update(i, |c| c.player_rotation_offset = c.rotation_offset);
            }
            // MinecartCommandBlock.interact on the client: the editor opens.
            if self.common.carts.iter().any(|c| {
                c.id as i32 == hit.id
                    && c.kind == sunrise_gameplay_common::game::minecarts::CartKind::CommandBlock
            }) {
                self.open_cart_command_block(hit.id as u32);
            }
            // Minecart.interact boards the cart and answers CONSUME on the client,
            // which ends the click without an arm swing. AbstractBoat.interact
            // answers SUCCESS there, which swings. Cushion.interact answers
            // CONSUME there too.
            if remote_object_swings(response, is_rideable_cart || is_cushion) {
                self.with_hand(hand, PresentationSideGameExt::swing_arm);
            }
            return true;
        }
        if let Some((i, distance, location)) = self.mob_hit()
            && self.before_block(distance)
        {
            let id = self.common.mobs[i].id as i32;
            self.send(interact_packet(id, location));
            if let Some(done) = self.entity_interaction(hand, id) {
                return done;
            }
            // ShearsItem.interactLivingEntity runs on this side too: a part that makes the mob
            // shearable shears this copy, and the server decides the rest.
            if self.predict_part_shear(i, hand) {
                return true;
            }
            // Player.interactOn: another player passes, and so does a mob
            // the held item means nothing to: the item is used instead.
            let player = self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.level_players.contains_key(&id));
            if !player && self.remote_mob_takes(i, hand) {
                self.remote_feed_particles(i, hand);
                // As for the objects above, only SUCCESS swings the hand: a
                // CONSUME ends the click without one.
                if self.remote_mob_swings(i, hand) {
                    self.with_hand(hand, PresentationSideGameExt::swing_arm);
                }
                return true;
            }
            if !player
                && self.with_hand(hand, |g| {
                    g.item_interact_mob(Some(sunrise_gameplay_common::game::InteractTarget {
                        id,
                        location,
                    }))
                })
            {
                return true;
            }
        }
        false
    }
    /// MultiPlayerGameMode.useItemOn for `hand`: whether the click ends there
    /// (the block reacts, or a block item is placed).
    pub fn remote_use_item_on(
        &mut self,
        hand: Hand,
        attempt: mod_api::InteractionAttempt,
        hit: Option<Hit>,
    ) -> bool {
        let held = self.with_hand(hand, |g| g.held().cloned());
        if let Some(hit) = hit.or_else(|| self.pick()) {
            use mod_api::InteractionResult as Answer;
            use mod_api::{LogicalSide, TriState};
            use sunrise_world::level::UseBranch;
            // MultiPlayerGameMode.useItemOn syncs the original slot before prediction.
            // A Part can select a different slot, but not for this request.
            self.remote_send_carried();
            // startPrediction reserves one sequence for the request, even if
            // the client does not apply a prediction.
            let seq = self.next_sequence();
            let packet = play::UseItemOn {
                hand: hand_id(hand),
                pos: hit.pos,
                face: i32::from(face_direction(hit.face)),
                cursor: std::array::from_fn(|axis| {
                    (hit.point[axis] - f64::from(hit.pos[axis])) as f32
                }),
                inside: hit.inside,
                world_border: false,
                sequence: seq,
            };
            let spectator = self.common.spectator();
            // The client and server can answer RightClickBlock differently (for
            // example, a server-only listener may override sneaking). Selection
            // comes from the registration snapshot, not the client's hook answer.
            let cauldron_click = self
                .client
                .remote
                .as_ref()
                .filter(|_| !spectator && self.has_server_cauldron_handler(hit.pos, held.as_ref()))
                .map(|remote| {
                    let selected = self.common.selected;
                    let main = self.common.stack_in(selected).cloned();
                    let off = self.common.offhand.clone();
                    let aim = self.remote_item_use_aim();
                    let swing = remote.interaction_swing;
                    move |sequence, item_after_block| PendingCauldron {
                        hand,
                        hit,
                        attempt,
                        selected,
                        main,
                        off,
                        aim,
                        swing,
                        sequence,
                        item_after_block,
                    }
                });
            // RightClickBlock on this client, where performUseItemOn starts,
            // for a spectator too (whose use only opens menus on the server):
            // it may force the block's use or the item's useOn, or answer for
            // both.
            let (mut canceled, mut use_block, mut use_item) =
                (None, TriState::Default, TriState::Default);
            match self.right_click_block(&hit, hand, held.as_ref()) {
                sunrise_gameplay_common::game::native_hooks::BlockClickAnswer::Go {
                    use_block: block,
                    use_item: item,
                } => (use_block, use_item) = (block, item),
                sunrise_gameplay_common::game::native_hooks::BlockClickAnswer::Canceled(answer) => {
                    canceled = Some(answer);
                }
            }
            // Before-block event, then the held item's first refusal. A
            // canceled PASS skips that Part but still goes on to the block.
            // A stale prediction sends the ordinary request and predicts nothing.
            if canceled.is_none()
                && !spectator
                && use_item.or(true)
                && let Some(stack) = held.as_ref()
            {
                let Ok(answer) = self.item_use_first(&hit, hand, stack) else {
                    self.send(S::UseItemOn(packet));
                    return true;
                };
                canceled = answer.filter(|answer| *answer != Answer::Pass);
            }
            // Player.isSecondaryUseActive with something in either hand
            // keeps the block from reacting by default; an explicit block
            // allow overrides that skip, and False always skips the block.
            let block_used =
                canceled.is_none() && use_block.or(!self.remote_secondary_use_skips_block());
            let item_forced = use_item == TriState::True;
            let item_used = canceled.is_none()
                && use_item.or(true)
                && (self.may_build()
                    || self.with_hand(hand, |g| g.can_place_on_in_adventure(hit.pos)))
                && held
                    .as_ref()
                    .is_none_or(|stack| self.items().get(&stack.item).is_some())
                && (item_forced
                    || held.as_ref().is_some_and(|stack| {
                        self.items().get(&stack.item).is_some_and(|info| {
                            !self
                                .common
                                .cooldowns
                                .is_on_cooldown(&sunrise_items::cooldowns::group(
                                    stack,
                                    info.use_cooldown.as_ref(),
                                ))
                        })
                    }));
            let [x, y, z] = hit.pos;
            let state = self.common.world.block(x, y, z);
            let block_answer = if !spectator && block_used {
                if let Some(answer) =
                    self.use_item_on_block(&hit, hand, held.as_ref(), mod_api::UsePhase::Block)
                {
                    Some(answer)
                } else {
                    let Ok(answer) = self.predict_block_part(
                        &hit,
                        state,
                        hand,
                        held.as_ref(),
                        UseBranch::ItemOn,
                    ) else {
                        self.send(S::UseItemOn(packet));
                        return true;
                    };
                    answer
                }
            } else {
                None
            };
            let vanilla_block = cauldron_click.is_none() && block_used && block_answer.is_none();
            // ShelfBlock.useItemOn consumes a nonempty main-hand click client-side
            // without predicting the inventory swap or swinging.
            let shelf_reacts = hand == Hand::Main
                && held.is_some()
                && vanilla_block
                && self.common.shelf_hit(&hit).is_some();
            let (result, changes) = if let Some(answer) = block_answer {
                let result = sunrise_gameplay_common::game::native_hooks::block_use_result(
                    answer,
                    LogicalSide::Client,
                );
                (
                    result.or((hand == Hand::Off).then_some(InteractionResult::Pass)),
                    DryUse::default(),
                )
            } else if shelf_reacts {
                (Some(InteractionResult::Consume), DryUse::default())
            } else if hand == Hand::Main && !spectator && vanilla_block {
                self.remote_block_shortcut(&hit, held.as_ref())
                    .map(|(result, changes)| (Some(result), changes))
                    .unwrap_or_else(|| {
                        // With no fallback Part, keep both vanilla methods
                        // on the same dry level as an ordinary click.
                        let split =
                            sunrise_gameplay_common::game::api_values::block_state::block_listens(
                                mod_api::HookId::UseWithoutItem,
                                state,
                            );
                        self.remote_block_template(
                            &hit,
                            state,
                            held.as_ref(),
                            split.then_some(UseBranch::ItemOn),
                            true,
                        )
                    })
            } else {
                // useItemOn runs for the off hand too (useWithoutItem doesn't):
                // RespawnAnchorBlock.useItemOn charges an anchor with its
                // glowstone, SUCCESS; the blocks of `client_block_answer` answer too.
                let charges = vanilla_block && self.common.anchor_takes(hit.pos, held.as_ref());
                let result = if charges {
                    InteractionResult::Success
                } else if let Some(answer) = (vanilla_block && !spectator)
                    .then(|| self.client_block_answer(&hit, held.as_ref()))
                    .flatten()
                {
                    answer
                } else {
                    InteractionResult::Pass
                };
                (Some(result), DryUse::default())
            };
            // Campfire dousing is an item-on-block behaviour, not a use
            // without the item. If it takes the click there is no fallback.
            let result = if !spectator
                && vanilla_block
                && !result.is_some_and(InteractionResult::consumes_action)
                && self.with_hand(hand, |g| g.predict_campfire_douse(&hit, held.as_ref(), seq))
            {
                Some(InteractionResult::Success)
            } else {
                result
            };
            // Only the main-hand block branch can select the empty method.
            // That method still sees the held stack, and has no broadcast phase.
            let mut empty_answer = None;
            let (result, changes) = if let Some(result) = result {
                (result, changes)
            } else {
                let Ok(answer) = self.predict_block_part(
                    &hit,
                    state,
                    hand,
                    held.as_ref(),
                    UseBranch::WithoutItem,
                ) else {
                    self.send(S::UseItemOn(packet));
                    return true;
                };
                empty_answer = answer;
                if let Some(answer) = answer {
                    (
                        sunrise_gameplay_common::game::native_hooks::block_use_result(
                            answer,
                            LogicalSide::Client,
                        )
                        .unwrap_or(InteractionResult::Pass),
                        DryUse::default(),
                    )
                } else {
                    let (result, changes) = self.remote_block_template(
                        &hit,
                        state,
                        held.as_ref(),
                        Some(UseBranch::WithoutItem),
                        true,
                    );
                    (result.unwrap_or(InteractionResult::Pass), changes)
                }
            };
            if let Some(click) = cauldron_click
                && canceled.is_none_or(|answer| !answer.consumes_action() && answer != Answer::Fail)
                && !result.consumes_action()
            {
                // A terminal client answer has no continuation and takes the
                // ordinary path below. Otherwise wait before the after-block
                // hooks, which can apply item effects of their own.
                if !self.client_packet_send_canceled(&S::UseItemOn(packet.clone()))
                    && let Some(remote) = &mut self.client.remote
                {
                    use sunrise_protocol::buf::Codec;
                    remote.cauldron_receipt = remote.cauldron_receipt.wrapping_add(1);
                    let receipt = remote.cauldron_receipt;
                    let mut body = sunrise_core::vanilla_bytes::Writer::new();
                    body.i64(receipt);
                    play::UseItemOn::write(&packet, &mut body);
                    // The envelope carries a receipt for the *same* block-use
                    // action. Do not expose it instead of the logical UseItemOn.
                    self.send_or_hold_play(
                        S::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
                            channel: super::super::cauldron_prediction::USE_CHANNEL.into(),
                            data: body.data,
                        }),
                        Some((receipt, click(seq, item_used))),
                    );
                }
                return true;
            }
            // AbstractCauldronBlock.useItemOn, either hand: CauldronInteractions
            // answers on the client as on the server and only the server acts,
            // so a bucket the cauldron takes is not used again (UseItem).
            // The main-hand path already asked above; only the off-hand fallback asks here.
            let result =
                if result.consumes_action() || spectator || !vanilla_block || hand == Hand::Main {
                    result
                } else {
                    match self.with_hand(hand, |game| game.cauldron_outcome(hit.pos, held.as_ref()))
                    {
                        sunrise_sim::cauldron::Outcome::Success => InteractionResult::Success,
                        sunrise_sim::cauldron::Outcome::Consume => InteractionResult::Consume,
                        sunrise_sim::cauldron::Outcome::Pass => result,
                    }
                };
            // useItemOn on this client, unless a sneak with something in hand
            // skips it: RedStoneOreBlock's dust, and the wick smoke of a lit
            // candle an empty hand snuffs (CandleBlock.useItemOn) or a lit
            // candle cake (CandleCakeBlock.useItemOn).
            if hand == Hand::Main && !spectator && vanilla_block {
                self.redstone_ore_particles(hit.pos);
                let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                let name = self.block_name(hit.pos);
                let lit = self
                    .common
                    .pack
                    .assets
                    .registry
                    .state(state)
                    .is_some_and(|s| s.properties.get("lit").map(String::as_str) == Some("true"));
                let snuffs = if name.ends_with("candle_cake") {
                    hit.point[1] - f64::from(hit.pos[1]) > 0.5
                } else {
                    name.ends_with("candle") && self.may_build()
                };
                if held.is_none() && lit && snuffs {
                    self.candle_smoke(hit.pos, state);
                }
            }
            let item_answer = if !spectator && !result.consumes_action() && item_used {
                self.remote_item_after_block(&hit, hand, held.as_ref())
            } else {
                None
            };
            let item_used =
                item_used && !spectator && !result.consumes_action() && item_answer.is_none();
            let reacts = result.consumes_action();
            let debug_stick = item_used
                && held
                    .as_ref()
                    .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
            // ArmorStandItem.useOn checks the space in startPrediction before
            // the packet goes out (the server may spawn the stand at once).
            let stand_fits = item_used && self.remote_armor_stand_fits(&hit, held.as_ref());
            self.send(S::UseItemOn(packet));
            // MultiPlayerGameMode.performUseItemOn: a spectator's use predicts
            // nothing and ends the click (CONSUME, no swing); the server opens
            // the block's menu. A cancelled RightClickBlock answers instead,
            // as for any player (below).
            if spectator && canceled.is_none() {
                return true;
            }
            // A cancelled RightClickBlock's answer, once the packet went out
            // (startPrediction sends it whatever the use): SUCCESS swings
            // here, and PASS goes on to useItem and the other hand.
            if let Some(answer) = canceled {
                return self.finish_remote_item_on(hand, answer);
            }
            // Only a predicted placement (predict_place) marks a cell whose
            // resend plays a place sound: an empty hand or a non-block item
            // places nothing (BlockItem.place is where the sound comes from).
            if let Some(r) = self.client.remote.as_mut() {
                r.use_target = Some(hit.pos);
            }
            if let Some(answer) = item_answer {
                return self.finish_remote_item_on(hand, answer);
            }
            if shelf_reacts {
                return true;
            }
            if reacts {
                self.predict_use(&changes, seq);
            }
            // LeverBlock.useWithoutItem on the client: the predicted pull puffs.
            if (reacts || debug_stick)
                && empty_answer.is_none()
                && (vanilla_block || block_answer == Some(Answer::TryWithEmptyHand))
            {
                if &*self.block_name(hit.pos) == "minecraft:lever" {
                    self.lever_puff(hit.pos);
                }
                self.open_command_block_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
                self.open_block_editor_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
            }
            if reacts {
                // MultiPlayerGameMode.useItemOn: SUCCESS SwingSource.CLIENT
                // (a chest, barrel, furnace, …). CONSUME does not swing.
                if result.should_swing() {
                    self.with_hand(hand, PresentationSideGameExt::swing_arm);
                }
                return true;
            }
            return item_used
                && self.remote_vanilla_item_on(&hit, hand, held.as_ref(), seq, stand_fits);
        }
        false
    }
    /// `MultiPlayerGameMode.performUseItemOn`'s local `ItemStack.useOn`, after
    /// the block and the item's hook passed. Publishing the block packet is
    /// the caller's job: a deferred item turn must not send it again.
    pub(in crate::game::client_side) fn remote_vanilla_item_on(
        &mut self,
        hit: &Hit,
        hand: Hand,
        held: Option<&ItemStack>,
        seq: i32,
        stand_fits: bool,
    ) -> bool {
        let Some(stack) = held else {
            return false;
        };
        let starter = ability::FIRESTARTER_LIGHT;
        match &*stack.item {
            // TntBlock.useItemOn primes TNT for another stack a part grants
            // `firestarter_light` (vanilla's answer false, as the server's `ignite_tnt` gives
            // it; the vanilla starters are predicted below), unless sneaking with hands full.
            item if !matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge")
                && &*self.block_name(hit.pos) == "minecraft:tnt"
                && self.stack_can_perform(stack, starter, false)
                && !(self.remote_shift_down()
                    && (self.common.stack_in(self.common.selected).is_some()
                        || self.common.offhand.is_some())) =>
            {
                self.with_hand(hand, PresentationSideGameExt::swing_arm);
                return true;
            }
            "minecraft:debug_stick" => {
                // The client answers SUCCESS and swings; only the server changes the block.
                self.with_hand(hand, PresentationSideGameExt::swing_arm);
                return true;
            }
            "minecraft:end_crystal" => {
                // EndCrystalItem.useOn: SUCCESS where the crystal fits (the
                // server spawns it), FAIL elsewhere. Either ends the click.
                if self.end_crystal_fits(hit.pos) {
                    self.with_hand(hand, PresentationSideGameExt::swing_arm);
                }
                return true;
            }
            "minecraft:brush" => {
                // BrushItem.useOn starts brushing (CONSUME, no swing); the
                // server brushes and the dust is this client's own.
                self.with_hand(hand, HeldItemsGameExt1::start_using_item);
                return true;
            }
            "minecraft:bone_meal"
                if self.with_hand(hand, |g| g.remote_bone_meal_succeeds(hit, stack)) =>
            {
                // BoneMealItem.useOn succeeds here; the server grows the plant.
                self.with_hand(hand, PresentationSideGameExt::swing_arm);
                return true;
            }
            "minecraft:armor_stand" if stand_fits => {
                // ArmorStandItem.useOn succeeds once the stand fits.
                self.with_hand(hand, PresentationSideGameExt::swing_arm);
                return true;
            }
            item if item.ends_with("_spawn_egg") => {
                // SpawnEggItem.useOn is FAIL when the mob cannot spawn, else
                // SUCCESS. No Item.use follows to spawn a second mob.
                if sunrise_sim::block_entity_item::item_entity_data(stack, self.common.res.items())
                    .is_some_and(|data| self.common.spawn_egg_allowed(&data.kind))
                {
                    self.with_hand(hand, PresentationSideGameExt::swing_arm);
                }
                return true;
            }
            "minecraft:flint_and_steel" | "minecraft:fire_charge" => {
                if self.predict_fire_starter(stack, hit, hand, seq) {
                    self.with_hand(hand, PresentationSideGameExt::swing_arm);
                }
                return true;
            }
            "minecraft:potion"
                if stack.potion() == Some("minecraft:water")
                    && hit.face != [0, -1, 0]
                    && self.named_block_in_tag(
                        "minecraft:convertible_to_mud",
                        &self.block_name(hit.pos),
                    )
                    && (self.may_build()
                        || self.with_hand(hand, |g| g.can_place_on_in_adventure(hit.pos))) =>
            {
                // PotionItem.useOn pours out the bottle and turns the block to
                // mud locally; sounds and splash particles come from the server.
                if let Some(mud) = self
                    .common
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:mud")
                {
                    let pos =
                        sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
                    self.predict_blocks(&[(pos, mud)], seq);
                }
                self.with_hand(hand, |g| {
                    g.filled_result(ItemStack::new("minecraft:glass_bottle", 1))
                });
                self.with_hand(hand, PresentationSideGameExt::swing_arm);
                return true;
            }
            _ => {}
        }
        if self.with_hand(hand, |g| g.predict_item_on_block(stack, hit))
            || self.predict_map_on_banner(stack, hit)
        {
            self.with_hand(hand, PresentationSideGameExt::swing_arm);
            return true;
        }
        if let Some(info) = self
            .items()
            .get(&stack.item)
            .filter(|info| info.block.is_some())
        {
            let consumable = info.consumable.is_some().then(|| info.clone());
            let placed = self.remote_can_place(hit, stack);
            // A consumable that could not be placed is used (Item.use), which
            // the server does for this same packet. A full player's FAIL still
            // ends the click. Adventure leaves the placing to the server.
            if placed.is_none()
                && self.may_build()
                && let Some(info) = &consumable
            {
                self.with_hand(hand, |g| g.client_start_eating(info));
            }
            if let Some(placed) = placed {
                self.predict_place(&placed, stack, seq);
                self.with_hand(hand, |g| g.consume_placed(stack));
                self.with_hand(hand, PresentationSideGameExt::swing_arm);
            }
            // BlockItem.place's FAIL also ends the click.
            return true;
        }
        false
    }
    /// Bounding box `[lo, hi]` and world origin `[x, y, z]` of a crosshair entity for
    /// `Minecraft.startUseItem` (including remote/resident leash knots, which have no API body).
    pub(in crate::game::client_side) fn use_entity_geometry(
        &self,
        entity: mod_api::EntityRef,
    ) -> Option<([[f64; 3]; 2], [f64; 3])> {
        let knot_block = self
            .client
            .remote_knots
            .get(&entity.id())
            .copied()
            .or_else(|| {
                self.common
                    .resident_entities
                    .knots
                    .iter()
                    .find(|k| k.id == entity.id())
                    .map(|k| k.fence)
            });
        if let Some(block) = knot_block {
            let bounds = crate::game::pick_entity::knot_bounds(block);
            let p = sunrise_sim::mobs::leash::knot_state(block).position;
            let (lo, hi) = (bounds.min_position(), bounds.max_position());
            return Some(([[lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]], [p.x, p.y, p.z]));
        }
        Some((
            self.entity_bounding_box(entity)?,
            self.entity_position(entity)?,
        ))
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::use_item_hand::UseItemHandGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
