//! A guest's flying kicks and the moves its actions hold, in their own module
//! so `hub.rs` keeps its size. The `handlePlayerPositionChange` collision
//! check is in `move_collision`.
use super::*;
impl Hub {
    /// `ServerGamePacketListenerImpl.tickPlayer`'s flying check: a player the
    /// last move left hovering for more than `getMaximumFlyingTicks` ticks is
    /// kicked.
    pub(super) fn tick_floating(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        let kick = self.guests.update(i, |g| {
            if g.client_is_floating && !g.sleeping && g.vehicle.is_none() && !g.dead {
                g.above_ground_ticks += 1;
                if g.above_ground_ticks > maximum_flying_ticks(PLAYER_GRAVITY) {
                    events.push(HostEvent::Warn(format!(
                        "{} was kicked for floating too long!",
                        g.name
                    )));
                    return Some(g.conn);
                }
            } else {
                g.client_is_floating = false;
                g.above_ground_ticks = 0;
            }
            None
        });
        if let Some(conn) = kick {
            self.kick(conn, translatable("multiplayer.disconnect.flying", &[]));
        }
        // The vehicle the player drives, by its own gravity: a boat may hover 160 ticks, a horse 80.
        let kick = self.guests.update(i, |g| {
            if g.vehicle_floating && g.vehicle.is_some() {
                g.above_ground_vehicle_ticks += 1;
                if g.above_ground_vehicle_ticks > maximum_flying_ticks(g.vehicle_gravity) {
                    events.push(HostEvent::Warn(format!(
                        "{} was kicked for floating a vehicle too long!",
                        g.name
                    )));
                    return Some(g.conn);
                }
            } else {
                g.vehicle_floating = false;
                g.above_ground_vehicle_ticks = 0;
            }
            None
        });
        if let Some(conn) = kick {
            self.kick(conn, translatable("multiplayer.disconnect.flying", &[]));
        }
    }

    /// A move still queued for guest `i`, which a packet the game handles
    /// after this pass must not overtake.
    pub(super) fn move_queued(&self, i: usize) -> bool {
        self.guests[i].inbox.iter().any(|entry| {
            let InboxItem::Packet(p, _) = entry else {
                return false;
            };
            matches!(
                p,
                S::MovePlayerPos(_)
                    | S::MovePlayerPosRot(_)
                    | S::MovePlayerRot(_)
                    | S::MovePlayerStatus(_)
            )
        })
    }
}

impl Hub {
    // ---- incoming ------------------------------------------------------

    /// The squared movement past 1.0E-5F (handlePlayerKnownMovement).
    fn moved_far(&self, i: usize, to: [f64; 3]) -> bool {
        let from = self.guests[i].position;
        let d: f64 = (0..3).map(|k| (to[k] - from[k]).powi(2)).sum();
        d > f64::from(1.0E-5_f32)
    }

    /// handlePlayerCommand's START_SPRINTING and STOP_SPRINTING: `setSprinting`, whose movement
    /// speed modifier the attributes then sync to the client (which takes the modifiers as sent:
    /// one without it would slow a sprinting client to a walk).
    fn set_sprinting(&mut self, i: usize, sprinting: bool) {
        self.guests.update(i, |g| {
            g.sprinting = sprinting;
            if let Some(m) = g.motion.as_mut() {
                let _ = m.set_sprint_modifier(sprinting);
            }
        });
    }

    pub(super) fn process_inbox(
        &mut self,
        i: usize,
        ctx: &mut TickCtx,
        events: &mut Vec<HostEvent>,
        begin: bool,
        held: &mut bool,
        spectator_source: &mut Option<String>,
    ) -> bool {
        let mut credits = Vec::new();
        // ServerGamePacketListenerImpl.tickPlayer ran between the last
        // tick's packets and these: resetPosition, and the moves it counted.
        if begin {
            self.guests.update(i, |g| {
                g.first_good = g.position;
                g.known_moves = g.received_moves;
                // ServerPlayer.tick: tickClientLoadTimeout.
                g.load_timeout = g.load_timeout.saturating_sub(1);
            });
        }
        // An action the game runs after this pass has a move queued behind it: the
        // move waits for the next tick, but the arm swing the client sent right after
        // the action is still handled with it, as vanilla handles both in this tick
        // (the swing starts the attack strength again, #7817).
        loop {
            if self.discard_closed_input(i) || self.holds_for_move(i) {
                if self.pause_for_move(i, &mut credits, events) {
                    return true;
                }
                break;
            }
            if *held
                && !matches!(
                    self.guests[i].inbox.front(),
                    Some(InboxItem::Packet(S::Punch(_), _) | InboxItem::Raw(_, _))
                )
            {
                break;
            }
            let Some(entry) = self.guests.update(i, |g| g.inbox.pop_front()) else {
                break;
            };
            let packet = match entry {
                InboxItem::Raw(delivery, credit) => {
                    if yield_raw(delivery, credit, events, &mut credits) {
                        return true;
                    }
                    continue;
                }
                InboxItem::Packet(packet, credit) => {
                    credits.extend(credit);
                    packet
                }
            };
            let conn = self.guests[i].conn;
            // ServerPlayer.resetLastActionTime: every handler of a player's
            // own action resets it, movement only when it moved
            // (handlePlayerKnownMovement), PlayerInput once loaded.
            let acted = match &packet {
                S::AcceptTeleportation(_)
                | S::PlayerAbilities(_)
                | S::MovePlayerStatus(_)
                | S::MovePlayerRot(_)
                | S::PlayerLoaded(_)
                | S::ChunkBatchReceived(_)
                | S::ClientInformation(_)
                | S::RecipeBookChangeSettings(_)
                | S::RecipeBookSeenRecipe(_)
                | S::ClientTickEnd(_)
                | S::DebugSubscriptionRequest(_)
                | S::KeepAlive(_)
                | S::Pong(_)
                | S::PingRequest(_)
                | S::UseItem(_) => false,
                S::MovePlayerPos(p) => {
                    !self.guests[i].won_game && self.moved_far(i, [p.x, p.y, p.z])
                }
                S::MovePlayerPosRot(p) => {
                    !self.guests[i].won_game && self.moved_far(i, [p.x, p.y, p.z])
                }
                S::PlayerInput(_) => self.guests[i].client_loaded(),
                _ => true,
            };
            if acted {
                {
                    let value = self.ticks;
                    self.guests
                        .update(i, |guest| guest.last_action_tick = value);
                }
            }
            let event_start = events.len();
            match packet {
                // hasClientLoaded: these handlers ignore the player's actions until
                // the client says the level around it is loaded (or 60 ticks pass),
                // and while it is dead.
                S::PlayerAction(_)
                | S::UseItemOn(_)
                | S::UseItem(_)
                | S::Attack(_)
                | S::Interact(_)
                | S::SpectatorAction(_)
                | S::PlayerCommand(_)
                    if !self.guests[i].client_loaded() => {}
                // handleAcceptTeleportPacket.
                S::AcceptTeleportation(p) => {
                    let g = &self.guests[i];
                    if p.id == g.teleport_counter && g.awaiting_teleport.is_none() {
                        self.kick(conn, invalid_movement());
                        continue;
                    }
                    if let Some((id, at)) = g.awaiting_teleport
                        && id == p.id
                    {
                        if p.x.is_nan()
                            || p.y.is_nan()
                            || p.z.is_nan()
                            || !p.yaw.is_finite()
                            || !p.pitch.is_finite()
                        {
                            self.kick(conn, invalid_movement());
                            continue;
                        }
                        self.guests.update(i, |g| {
                            g.position = at;
                            g.awaiting_teleport = None;
                            g.pending_travel_velocity = None;
                        });
                        // The confirmation is then an ordinary report of where the client
                        // is: its own position goes through the speed test, the collided
                        // replay and the moved-wrongly test, from the target just snapped to.
                        let (to, rot) = ([p.x, p.y, p.z], (p.yaw, p.pitch));
                        self.position_change(i, to, rot, 0, Hearing::Confirmation, ctx, events);
                    }
                }
                // handlePlayerAbilities: the client flies only if it may.
                S::PlayerAbilities(p) => {
                    self.guests.update(i, |g| {
                        let flying = p.flags & 2 != 0 && g.abilities.may_fly;
                        // The change is the game's to announce (the mods may refuse it).
                        if flying != g.abilities.flying {
                            g.abilities.flying = flying;
                            events.push(HostEvent::Flight { conn, flying });
                        }
                    });
                }
                S::MovePlayerPos(p) => {
                    self.move_player(i, Some([p.x, p.y, p.z]), None, p.flags, ctx, events);
                }
                S::MovePlayerPosRot(p) => {
                    let (pos, rot) = ([p.x, p.y, p.z], (p.yaw, p.pitch));
                    self.move_player(i, Some(pos), Some(rot), p.flags, ctx, events);
                }
                S::MovePlayerRot(p) => {
                    self.move_player(i, None, Some((p.yaw, p.pitch)), p.flags, ctx, events);
                }
                S::MovePlayerStatus(p) => {
                    self.move_player(i, None, None, p.flags, ctx, events);
                }
                S::CustomPayload(p) if p.channel == sunrise_protocol::carry_age::CHANNEL => {
                    // A payload that does not decode is no hint: it is only where a search starts.
                    if let Ok(hint) = sunrise_protocol::carry_age::decode(&p.data) {
                        self.guests
                            .update(i, |g| g.carry_hint = Some((hint.collider, hint.ticks)));
                    }
                }
                S::CustomPayload(p)
                    if p.channel == sunrise_protocol::passage_sync::CROSS_CHANNEL =>
                {
                    *held = self.passage_leg(i, p, ctx, events);
                }
                // handleDebugSubscriptionRequest.
                S::DebugSubscriptionRequest(p) => {
                    self.guests
                        .update(i, |g| g.debug_subscriptions = p.subscriptions);
                }
                // handleAcceptPlayerLoad: markClientLoaded.
                S::PlayerLoaded(_) => {
                    self.guests.update(i, |g| {
                        g.loaded = true;
                        g.loading = false;
                        g.load_timeout = 0;
                        events.push(HostEvent::PlayerLoaded(conn));
                    });
                }
                // PlayerChunkSender.onChunkBatchReceivedByClient.
                S::ChunkBatchReceived(p) => {
                    self.guests.update(i, |g| {
                        g.unacknowledged = g.unacknowledged.saturating_sub(1);
                        g.chunks_per_tick = if p.chunks_per_tick.is_nan() {
                            0.01
                        } else {
                            p.chunks_per_tick.clamp(0.01, 64.0)
                        };
                        if g.unacknowledged == 0 {
                            g.batch_quota = 1.0;
                        }
                        g.max_unacknowledged = 10;
                    });
                }
                // ServerCommonPacketListenerImpl.handlePong has no vanilla side effects.
                S::Pong(p) => {
                    if Self::observes_play_pongs(&self.guests[i]) {
                        events.push(HostEvent::PlayerPong { conn, id: p.id });
                        events.push(HostEvent::InputCredits(credits));
                        return true;
                    }
                }
                // ServerGamePacketListenerImpl.handlePingRequest: echo on this connection.
                S::PingRequest(p) => self.send_to(conn, &C::PongResponse(p)),
                // handleAttack is the game's too: a move queued behind an attack would move
                // the player (or land them on the ground, losing a falling attack's critical
                // hit) before the game checks their reach and state.
                S::Attack(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::Attack(p))));
                    *held = self.move_queued(i);
                }
                S::Punch(p) => events.push(HostEvent::Packet(conn, Box::new(S::Punch(p)))),
                S::ClientTickEnd(_) => {
                    // handleClientTickEnd: no move this tick, no known movement.
                    self.guests.update(i, |g| {
                        if !g.position_this_tick {
                            g.known_movement = [0.0; 3];
                        }
                        g.position_this_tick = false;
                    });
                }
                // ServerGamePacketListenerImpl.handleRecipeBookSeenRecipePacket:
                // the display the player looked at stops being a new one, so
                // the next `RecipeBookAdd` it is sent does not animate.
                S::RecipeBookSeenRecipe(p) => {
                    let seen = self
                        .sync
                        .advancements
                        .recipe_of_display(p.recipe)
                        .map(str::to_owned);
                    if let Some(id) = seen {
                        self.guests
                            .update(i, |g| g.recipe_book.remove_highlight(&id));
                    }
                }
                // ServerGamePacketListenerImpl.handleSeenAdvancements: the tab the
                // player opened, kept when it names a root with a display
                // (`PlayerAdvancements.setSelectedTab` leaves the tab empty otherwise),
                // and answered with the tab it now keeps when that changed. An id the
                // server has no advancement for changes nothing.
                S::SeenAdvancements(p) => {
                    if p.action == 0
                        && let Some(id) = p.tab
                        && let Some(keep) = self.sync.advancements.selectable(&id)
                    {
                        let tab = keep.then_some(id);
                        self.guests.update(i, |g| {
                            if g.selected_tab != tab {
                                g.selected_tab = tab.clone();
                                g.send(&C::SelectAdvancementsTab(play::SelectAdvancementsTab {
                                    tab,
                                }));
                            }
                        });
                    }
                }
                // ServerGamePacketListenerImpl.handleRecipeBookChangeSettingsPacket.
                S::RecipeBookChangeSettings(p) => {
                    if let Some(book) = crate::recipe_book::BookType::ALL.get(p.book_type as usize)
                    {
                        self.guests.update(i, |g| {
                            let settings = &mut g.recipe_book;
                            let changed = settings.open[book.index()] != p.open
                                || settings.filtering[book.index()] != p.filtering;
                            settings.open[book.index()] = p.open;
                            settings.filtering[book.index()] = p.filtering;
                            if changed {
                                events.push(HostEvent::RecipeBookSettings {
                                    conn,
                                    book: p.book_type,
                                    open: p.open,
                                    filtering: p.filtering,
                                });
                            }
                        });
                    }
                }
                S::ClientInformation(info) => {
                    let effective = effective_view_distance(self.config.view_distance);
                    let (was_hat, hat, uuid) = self.guests.update(i, |g| {
                        // PlayerModelPart.HAT (bit 6): the tab list shows the hat layer.
                        let was_hat = g.info.model_customisation & 0x40 != 0;
                        g.view_distance = i32::from(info.view_distance).clamp(2, effective);
                        let old_info = self.mods_loaded.then(|| Box::new(g.info.clone()));
                        g.info = info;
                        if let Some(old) = old_info {
                            let new = Box::new(g.info.clone());
                            events.push(HostEvent::ClientInformation(conn, old, new));
                        }
                        g.view_dirty = true;
                        let hat = g.info.model_customisation & 0x40 != 0;
                        (was_hat, hat, g.uuid)
                    });
                    // ServerGamePacketListenerImpl.handleClientInformation: UPDATE_HAT to everyone.
                    if hat != was_hat {
                        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                            actions: 128,
                            entries: vec![play::PlayerInfo {
                                uuid,
                                show_hat: Some(hat),
                                ..Default::default()
                            }],
                        }));
                    }
                }
                S::PlayerInput(p) => {
                    self.guests.update(i, |g| {
                        g.input = p.flags;
                        if g.client_loaded() {
                            // The events only tell the mods: nothing is queued with none loaded.
                            let listening = self.mods_loaded;
                            if listening {
                                events.push(HostEvent::Input {
                                    conn,
                                    flags: i32::from(p.flags),
                                });
                            }
                            let sneaking = p.flags & 32 != 0;
                            if g.sneaking != sneaking {
                                g.sneaking = sneaking;
                                if listening {
                                    events.push(HostEvent::Sneak { conn, sneaking });
                                }
                            }
                        }
                    });
                    // handlePlayerInput only records keys and updates crouching.
                    // Sprinting persists after the sprint key is released (or
                    // starts with a double tap); PlayerCommand changes it.
                }
                // OPEN_INVENTORY (the ridden mount's inventory) and
                // START_FALL_FLYING (checked against the guest's chest slot
                // and effects) are the game's.
                // START_RIDING_JUMP and STOP_RIDING_JUMP: the vehicle's
                // PlayerRideableJumping (handleStartJump / handleStopJump).
                S::PlayerCommand(p) if matches!(p.action, 3 | 4) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                }
                S::PlayerCommand(p) if matches!(p.action, 5 | 6) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                }
                S::PlayerCommand(p) => match p.action {
                    1 | 2 => {
                        let sprinting = p.action == 1;
                        let changed = self.guests[i].sprinting != sprinting;
                        self.set_sprinting(i, sprinting);
                        // The game tells the mods of a change.
                        if changed {
                            events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                        }
                    }
                    // STOP_SLEEPING: the game wakes the guest after the mods hear it
                    // (`stopSleepInBed`), and the rest of this turn waits for that wake:
                    // a move sent after it must not run while the guest still sleeps.
                    0 => {
                        events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                        *held = true;
                    }
                    _ => {}
                },
                S::Chat(c) => self.handle_chat(i, c, events),
                S::ChatAck(p) => self.chat_ack(i, p.offset, events),
                S::ChatSessionUpdate(p) => self.chat_session_update(i, p, events),
                // handlePlayerAction is the game's, run after this pass, and a move
                // queued behind it waits for the next tick, as vanilla handles the
                // packets in order. For a dig starting, stopping or finishing, the
                // move is checked against the block the player just broke and falls
                // through, so it would be sent back. A drop, a RELEASE_USE_ITEM (a
                // bow's arrow, a trident, a spear) and a stab use the player's
                // position and known movement now (#8423): shot after the move, an
                // arrow left the position of the tick after the release, a tick
                // ahead of the camera's instead of a tick behind it. A STAB (8) does
                // not hold the move: held a tick, the jump the move carries
                // (handlePlayerPositionChange's jumpFromGround) ran a tick after the
                // Lunge's impulse, and the server took two moves in the next tick
                // (lunge_motion, every jump phase).
                S::PlayerAction(p) => {
                    let stab = p.action == 8;
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerAction(p))));
                    *held = !stab && self.move_queued(i);
                }
                // handleUseItem uses the player's position and known movement now:
                // a later move must not overtake a throw before the game runs it.
                S::UseItem(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::UseItem(p))));
                    *held = self.move_queued(i);
                }
                // handleUseItemOn is the game's too, and BlockItem.canPlace checks
                // the player's box where they clicked from (Level.isUnobstructed), as
                // their client's prediction did. After the move behind it, a player
                // jumping up a pillar was already clear of the cell their client had
                // refused: the block went down without the place sound that only the
                // placer's client plays (BlockItem.place's playSound(player, ...)).
                S::UseItemOn(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::UseItemOn(p))));
                    *held = self.move_queued(i);
                }
                S::CustomPayload(p)
                    if p.channel
                        == sunrise_protocol::packets::custom_payload::CAULDRON_USE_CHANNEL =>
                {
                    events.push(HostEvent::Packet(conn, Box::new(S::CustomPayload(p))));
                    *held = self.move_queued(i);
                }
                // handleInteract sets shift in the game before using the entity. Finish
                // that turn before a later PlayerInput can overwrite the key state.
                // As for attacks, a move queued behind it waits for the next tick.
                S::Interact(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::Interact(p))));
                    *held = self.move_queued(i);
                    events.push(HostEvent::InputCredits(credits));
                    return true;
                }
                // handleMoveVehicle tests the pending teleport before moving the vehicle.
                // A later confirmation or player move must not run ahead of that test.
                packet @ S::MoveVehicle(_) => {
                    events.push(HostEvent::Packet(conn, Box::new(packet)));
                    events.push(HostEvent::InputCredits(credits));
                    return true;
                }
                // A UUID teleport can change the world the next packet runs in.
                packet @ S::TeleportToEntity(_) if self.guests[i].spectator() => {
                    spectator_source.get_or_insert_with(|| self.config.dimension.clone());
                    events.push(HostEvent::Packet(conn, Box::new(packet)));
                    events.push(HostEvent::InputCredits(credits));
                    return true;
                }
                // ContainerInput's codec (ByIdMap, OutOfBoundsStrategy.ZERO) reads an
                // unknown click type as PICKUP.
                S::ContainerClick(mut p) => {
                    if !(0..=6).contains(&p.input) {
                        p.input = 0;
                    }
                    events.push(HostEvent::Packet(conn, Box::new(S::ContainerClick(p))));
                }
                // ServerboundSelectBundleItemPacket's constructor: no index below -1.
                S::SelectBundleItem(p) if p.index < -1 => {
                    let reason = format!("Invalid selectedItemIndex: {}", p.index);
                    self.kick(conn, literal(&reason));
                }
                other => events.push(HostEvent::Packet(conn, Box::new(other))),
            }
            if events[event_start..]
                .iter()
                .any(|event| matches!(event, HostEvent::AcceptedMove(..)))
            {
                // Fall and landing can write to the level or move the player again. A
                // subsequent packet must read those writes, not just the accepted position.
                events.push(HostEvent::InputCredits(credits));
                return true;
            }
        }
        events.push(HostEvent::InputCredits(credits));
        false
    }

    /// ServerGamePacketListenerImpl.handleMovePlayer.
    fn move_player(
        &mut self,
        i: usize,
        pos: Option<[f64; 3]>,
        rot: Option<(f32, f32)>,
        flags: u8,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) {
        let conn = self.guests[i].conn;
        // containsInvalidValues, on the packet's values (0 where it has none).
        let (yaw, pitch) = rot.unwrap_or_default();
        if pos.is_some_and(|p| p.iter().any(|v| v.is_nan()))
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            self.kick(conn, invalid_movement());
            return;
        }
        let repeated = self.guests.update(i, |g| {
            if pos.is_some() {
                if g.position_this_tick {
                    return true;
                }
                g.position_this_tick = true;
            }
            false
        });
        if repeated {
            self.kick(conn, invalid_movement());
            return;
        }
        let g = &self.guests[i];
        // handleMovePlayer checks packet validity (including repeated positions) even
        // during the credits, but does not apply any part of a valid move then.
        if g.won_game || g.landing || !g.client_loaded() {
            return;
        }
        let rot = rot.unwrap_or((g.yaw, g.pitch));
        if self.update_awaiting_teleport(i) {
            // Resend with the old look, then snap the report's angles (the player's
            // current angles for a look-less packet) with Entity.absSnapRotationTo.
            self.guests.update(i, |g| {
                (g.yaw, g.pitch) = (wrap_degrees(rot.0), wrap_degrees(rot.1).clamp(-90.0, 90.0));
            });
            return;
        }
        let to = pos.unwrap_or(self.guests[i].position);
        self.position_change(i, to, rot, flags, Hearing::Heard, ctx, events);
    }

    /// `ServerGamePacketListenerImpl.updateAwaitingTeleport`: while waiting for an ack,
    /// resend the correction once more than 20 server ticks have elapsed.
    pub fn update_awaiting_teleport(&mut self, i: usize) -> bool {
        let guest = &self.guests[i];
        let Some((_, at)) = guest.awaiting_teleport else {
            return false;
        };
        if self.ticks - guest.teleport_tick > 20 {
            self.send_position_correction(i, Some(at), 0, [0.0; 3], guest.pending_travel_velocity);
        }
        true
    }

    /// ServerGamePacketListenerImpl.handlePlayerPositionChange.
    pub(super) fn position_change(
        &mut self,
        i: usize,
        to: [f64; 3],
        (yaw, pitch): (f32, f32),
        flags: u8,
        hearing: Hearing,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) {
        let (frozen, (check, elytra_check)) = (self.ticking.1, self.movement_checks);
        let to = [
            to[0].clamp(-3.0E7, 3.0E7),
            to[1].clamp(-2.0E7, 2.0E7),
            to[2].clamp(-3.0E7, 3.0E7),
        ];
        let (yaw, pitch) = (wrap_degrees(yaw), wrap_degrees(pitch));
        // A carried player's report is measured in the collider's frame, and judged in it.
        let plan = (hearing != Hearing::Confirmation)
            .then(|| self.carry_plan(i, to))
            .flatten();
        let reference = self.carry_speed_delta(i, to, plan.as_ref());
        let begin = self.guests.update(i, |g| {
            let owner = self.singleplayer_owner == Some(g.uuid);
            // A passenger only turns: its vehicle carries it (absSnapTo where it is).
            if g.vehicle.is_some() {
                (g.yaw, g.pitch) = (yaw, pitch);
                g.release_tickets();
                return std::ops::ControlFlow::Break(false);
            }
            let d = reference.unwrap_or_else(|| [0, 1, 2].map(|k| to[k] - g.first_good[k]));
            let moved = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if g.sleeping {
                if moved > 1.0 {
                    (g.yaw, g.pitch) = (yaw, pitch);
                    return std::ops::ControlFlow::Break(true);
                }
                return std::ops::ControlFlow::Break(false);
            }
            // TickRateManager.runsNormally: not frozen (a frozen game's steps
            // are not counted either; Sunrise's hub does not see them).
            if !frozen {
                g.received_moves += 1;
                let mut packets = g.received_moves - g.known_moves;
                if packets > 5 {
                    packets = 1;
                }
                // shouldCheckPlayerMovement (never the single-player owner).
                if !owner && check && (!g.fall_flying || elytra_check) {
                    let limit = if g.fall_flying { 300.0 } else { 100.0 };
                    // `movedDist - expectedDist` (getDeltaMovement): the velocity
                    // the server last simulated for the guest is expected
                    // movement, not a violation — a landing glide keeps its speed.
                    let expected = g.motion.as_ref().map_or(0.0, |m| {
                        let v = m.velocity;
                        v.x * v.x + v.y * v.y + v.z * v.z
                    });
                    if moved - expected > limit * f64::from(packets) {
                        events.push(HostEvent::Warn(format!(
                            "{} moved too quickly! {},{},{}",
                            g.name,
                            sunrise_core::java_number::double_to_string(d[0]),
                            sunrise_core::java_number::double_to_string(d[1]),
                            sunrise_core::java_number::double_to_string(d[2])
                        )));
                        return std::ops::ControlFlow::Break(true);
                    }
                }
            }
            std::ops::ControlFlow::Continue(g.position)
        });
        let start = match begin {
            std::ops::ControlFlow::Continue(start) => start,
            std::ops::ControlFlow::Break(resend) => {
                if resend {
                    // A carried player's refusal puts it back where the server has it on its
                    // collider, and is counted like any other.
                    let carried = self.guests[i].on_collider;
                    if carried.is_some() && reference.is_some() {
                        self.carry_rejected(i);
                    }
                    let back = self.guests[i].carry_correction;
                    self.guests.update(i, |g| g.carry_correction = None);
                    self.resend_teleport(i, back);
                    self.keep_carried(i, carried, back);
                }
                return;
            }
        };
        // The confirmation of a teleport (a carried player's correction included) is committed in
        // place against the world as it stands.
        let (wrongly, refused, stands) = if hearing == Hearing::Confirmation {
            self.collide(i, to, ctx)
        } else {
            self.collide_carried(i, to, ctx, plan)
        };
        if wrongly {
            let line = format!("{} moved wrongly!", self.guests[i].name);
            events.push(HostEvent::Warn(line));
        }
        if refused {
            self.guests.update(i, |g| (g.yaw, g.pitch) = (yaw, pitch));
            // A carried player goes back to where the server has it on its collider now.
            let back = self.guests[i].carry_correction.unwrap_or(start);
            self.guests.update(i, |g| g.carry_correction = None);
            let carried = self.guests[i].on_collider;
            self.resend_teleport(i, Some(back));
            self.keep_carried(i, carried, Some(back));
            return;
        }
        // A carried player is where the server puts it: its local position under the
        // collider's transform now, not the position in the frame the client held.
        let committed = self.guests[i].carry_commit;
        self.guests.update(i, |g| g.carry_commit = None);
        let to = committed.unwrap_or(to);
        // clientIsFloating, from the move that is accepted: no upward-or-level
        // step onto something, nothing but air around the body, and nothing
        // that lets the player hover (the server's allow-flight, mayfly,
        // levitation, gliding, riptide).
        let floating = ctx.collision.is_some() && {
            let g = &self.guests[i];
            to[1] - g.position[1] >= -0.03125
                && !stands
                && !g.spectator()
                && !self.config.allow_flight
                && !g.creative()
                && g.effects.get("levitation").is_none()
                && !g.fall_flying
                && !g
                    .motion
                    .as_ref()
                    .is_some_and(|m| m.auto_spin_attack_ticks > 0)
                && self.no_blocks_around(i, to, ctx.world)
        };
        // handlePlayerPositionChange: a client that says it is on the ground while the last move
        // the server resolved came down onto nothing is sent the blocks under it again, so a
        // client that missed a block change cannot stand on air for good.
        if flags & 1 != 0 && !self.guests[i].rests_on_ground {
            self.force_send_player_support_blocks(i, ctx);
        }
        let accepted = Accepted {
            to,
            yaw,
            pitch,
            flags,
            stands,
            floating,
        };
        self.accept_move(i, accepted, hearing, events);
    }

    /// `ServerGamePacketListenerImpl.noBlocksAround`: every block of the
    /// player's box grown by 0.0625 and stretched 0.55 down is air. Vanilla
    /// loads a chunk it reads; a chunk not loaded here yet (a player's first
    /// move after joining, before the spawn chunks are generated) is not
    /// known to be air, so it never makes the player float.
    pub(super) fn no_blocks_around(&self, i: usize, at: [f64; 3], world: &World) -> bool {
        let d = self.guests[i].dimensions();
        let half = f64::from(d.width) / 2.0 + 0.0625;
        let lo = [at[0] - half, at[1] - 0.0625 - 0.55, at[2] - half];
        let hi = [
            at[0] + half,
            at[1] + f64::from(d.height) + 0.0625,
            at[2] + half,
        ];
        // A moving collider's box under the feet holds the player up as a block does: it is not
        // floating on one, and the colliders are where they stand now, as the player is committed.
        no_blocks_in(world, lo, hi, &self.encoder)
            && !sunrise_world::moving_colliders::overlaps(
                sunrise_world::entity_obstruction::EntityIdentity(self.guests[i].entity_id as u64),
                sunrise_core::geometry::Aabb::new(
                    sunrise_core::geometry::Vec3::new(lo[0], lo[1], lo[2]),
                    sunrise_core::geometry::Vec3::new(hi[0], hi[1], hi[2]),
                ),
            )
    }

    /// `ServerGamePacketListenerImpl.forceSendPlayerSupportBlocks`: the blocks just under the
    /// player's box, at the box's four corners a hundred-thousandth below its floor, each once.
    /// A client whose block below went missing without it noticing keeps standing on that block
    /// on the server, so the server tells it what is there (`ClientboundBlockUpdatePacket`, and
    /// the block entity's update tag where it has one, `BlockEntity.getUpdatePacket`). No more
    /// than once every 200 ticks of the connection, so a client that stands on air for good is
    /// not sent four packets four times a second.
    pub(super) fn force_send_player_support_blocks(&mut self, i: usize, ctx: &TickCtx) {
        let tick = self.ticks.saturating_sub(self.guests[i].joined_tick);
        if tick < self.guests[i].support_blocks_sent.saturating_add(200) {
            return;
        }
        {
            let value = tick;
            self.guests
                .update(i, |guest| guest.support_blocks_sent = value);
        }
        let conn = self.guests[i].conn;
        let g = &self.guests[i];
        let d = g.dimensions();
        let half = f64::from(d.width) / 2.0;
        let at = g.position;
        // `box.minY - 9.999999747378752E-6`, which is (double) 1.0E-5f.
        let y = at[1] - f64::from(1.0E-5_f32);
        let cell = |v: f64| v.floor() as i32;
        let mut positions: Vec<[i32; 3]> = Vec::with_capacity(4);
        for (x, z) in [
            (at[0] - half, at[2] - half),
            (at[0] + half, at[2] - half),
            (at[0] - half, at[2] + half),
            (at[0] + half, at[2] + half),
        ] {
            let pos = [cell(x), cell(y), cell(z)];
            if !positions.contains(&pos) {
                positions.push(pos);
            }
        }
        for pos @ [x, y, z] in positions {
            self.send_authoritative_block(conn, ctx.world, pos);
            let Some(entity) = ctx.world.block_entities.get(&(x, y, z)) else {
                continue;
            };
            let state = ctx.world.block(x, y, z);
            let Some(tag) = self.encoder.update_tag((x, y, z), state.0, entity) else {
                continue;
            };
            let Some(kind) = self.encoder.entity_type(state.0) else {
                continue;
            };
            self.send_to(
                conn,
                &C::BlockEntityData(play::BlockEntityData { pos, kind, tag }),
            );
        }
    }
}
