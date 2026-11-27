//! The hub's half of a guest's crossing of a passage within its level (#3775,
//! docs/modding/src/how-to/passages.md): the move to where the client crossed from, checked
//! as any move; the position packet that accepts the crossing; and a refusal. The game finds
//! the passage, asks the mods and works out where the player comes out, in between.
use super::*;
use mod_api::PassageCrossRefusal as Refusal;
use sunrise_protocol::packets::configuration::CustomPayload;
use sunrise_protocol::passage_sync::{self, Message};

/// How far from the nearest point of the opening a crossing may start, in blocks: Immersive
/// Portals' `getDistanceToNearestPointInPortal` limit.
pub const REACH: u32 = 20;

/// The server's refusals as they travel: a refusal's code is its place here.
const REFUSALS: [Refusal; 12] = [
    Refusal::UnknownPassage,
    Refusal::Pending,
    Refusal::NotLoaded,
    Refusal::Dead,
    Refusal::Sleeping,
    Refusal::Riding,
    Refusal::MoveRefused,
    Refusal::TooFar { limit: REACH },
    Refusal::NearEndNotTracked,
    Refusal::FarEndNotTracked,
    Refusal::Vetoed,
    Refusal::FarLevel,
];

/// Where a crossing starts on the server: the client's `from` clamped as a move's position
/// is (`ServerGamePacketListenerImpl.clampHorizontal`, `clampVertical`).
pub fn crossing_source(from: [f64; 3]) -> [f64; 3] {
    [
        from[0].clamp(-3.0E7, 3.0E7),
        from[1].clamp(-2.0E7, 2.0E7),
        from[2].clamp(-3.0E7, 3.0E7),
    ]
}

/// The refusal's code on `sunrise:passage_sync`.
pub fn refusal_code(refusal: Refusal) -> u8 {
    REFUSALS.iter().position(|r| *r == refusal).unwrap_or(0) as u8
}

/// The refusal a code names; none for a code this build does not know.
pub fn refusal_named(code: u8) -> Option<Refusal> {
    REFUSALS.get(usize::from(code)).copied()
}

impl Hub {
    /// A guest's `sunrise:passage_cross` payload, in its place among the guest's packets:
    /// the move to where it crossed from is checked as a move packet would be (vanilla's
    /// numbers, from the server's last good position, on the ground the server has it on),
    /// though it is no client tick's move. Passed, the crossing goes to the game and true is
    /// returned: the rest of the guest's packets wait for the next tick. Refused, the
    /// client is told why and put back where the server has the player. Malformed or not
    /// a number: kicked, as a move with invalid values is.
    pub(super) fn passage_leg(
        &mut self,
        i: usize,
        payload: CustomPayload,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        let conn = self.guests[i].conn;
        let cross = match passage_sync::decode_cross(&payload.data) {
            Ok(cross) if !cross.from.iter().any(|v| v.is_nan()) => cross,
            _ => {
                self.kick(conn, invalid_movement());
                return false;
            }
        };
        if let Some(why) = self.passage_refusal(i) {
            self.refuse_passage(i, cross.id, why);
            return false;
        }
        // No client tick's move: the player's ground flag and look stay as the server has them.
        let g = &self.guests[i];
        let flags = u8::from(g.on_ground);
        let rot = (g.yaw, g.pitch);
        // The caller reads the position at once, so the report is committed in place, never held.
        self.position_change(i, cross.from, rot, flags, Hearing::Crossing, ctx, events);
        let g = &self.guests[i];
        if g.awaiting_teleport.is_some() || g.position != crossing_source(cross.from) {
            // The move check sent its own correction, or kept the player where they were.
            if g.awaiting_teleport.is_none() {
                self.resend_teleport(i, None);
            }
            self.tell_refusal(i, cross.id, Refusal::MoveRefused);
            return false;
        }
        events.push(HostEvent::Packet(conn, Box::new(S::CustomPayload(payload))));
        // Nothing of the guest's overtakes the game's decision, a second crossing least of
        // all: it would move the player before the first is decided from its own `from`.
        true
    }

    /// Why guest `i` may not cross a passage now, whatever the passage: the checks before
    /// the move to where it crossed from, which the game makes again after the mods' veto.
    pub fn passage_refusal(&self, i: usize) -> Option<Refusal> {
        let g = &self.guests[i];
        if g.dead {
            Some(Refusal::Dead)
        } else if g.landing || !g.client_loaded() {
            Some(Refusal::NotLoaded)
        } else if g.awaiting_teleport.is_some() {
            Some(Refusal::Pending)
        } else if g.vehicle.is_some() {
            Some(Refusal::Riding)
        } else if g.sleeping {
            Some(Refusal::Sleeping)
        } else {
            None
        }
    }

    /// The game accepted guest `i`'s crossing: the server moves the player to `to`, turns
    /// its look by `turn` degrees and its motion with `motion`, starts its move checks again
    /// from there (`ServerGamePacketListenerImpl.resetPosition`, as `ServerPlayer.teleport`
    /// and Immersive Portals do), and sends a position packet that is all relative and zero:
    /// the client, already there, stays put and keeps its velocity. Moves until its
    /// confirmation wait, as for any teleport, and the confirmation is checked from `to`.
    /// The fall distance and the flying ticks go on.
    pub fn follow_passage(
        &mut self,
        i: usize,
        to: [f64; 3],
        turn: f32,
        motion: impl FnOnce([f64; 3]) -> [f64; 3],
    ) {
        self.guests.update(i, |g| {
            g.yaw = wrap_degrees(g.yaw + turn);
            g.head_yaw = g.yaw;
            g.first_good = to;
            if let Some(m) = g.motion.as_mut() {
                let [x, y, z] = motion([m.velocity.x, m.velocity.y, m.velocity.z]);
                m.velocity = sunrise_core::geometry::Vec3::new(x, y, z);
            }
        });
        use play::relative::{DELTA_X, DELTA_Y, DELTA_Z, ROTATE_DELTA, X, X_ROT, Y, Y_ROT, Z};
        let all = X | Y | Z | Y_ROT | X_ROT | DELTA_X | DELTA_Y | DELTA_Z | ROTATE_DELTA;
        self.teleport_relative(i, Some(to), all, [0.0; 3]);
    }

    /// Refuses guest `i`'s crossing of passage `id`: the client is told `why`, then put back
    /// where the server has the player, as Immersive Portals does (`teleportEntityGeneral`
    /// to its own position). The flying ticks go on.
    pub fn refuse_passage(&mut self, i: usize, id: u64, why: Refusal) {
        self.tell_refusal(i, id, why);
        self.resend_teleport(i, None);
    }

    fn tell_refusal(&self, i: usize, id: u64, why: Refusal) {
        let reason = refusal_code(why);
        self.guests[i].send_passage_sync(&Message::Refused { id, reason });
    }
}

impl Hub {
    /// ServerLevel.removePlayerImmediately: remove the old body and its pairs, not
    /// the connection or the tab-list entry. `respawn` places the new body later.
    pub fn remove_player_for_respawn(&mut self, i: usize) {
        let id = self.guests.update(i, |g| {
            g.stop_riding();
            g.placed = false;
            g.respawning = true;
            g.pending_move = None;
            super::super::chunk_watch::unwatch_all(&mut self.chunk_watches, g);
            for id in g.tracker.ids() {
                g.send(&C::RemoveEntities(play::RemoveEntities { ids: vec![id] }));
            }
            g.tracker.clear();
            g.sent_chunks.clear();
            g.chunk_queue.clear();
            g.viewed.forget_views();
            g.viewed.own.reset();
            g.entity_id
        });
        self.respawned(id);
    }

    /// ServerLevel.removePlayerImmediately, then addRespawnedPlayer: the other players
    /// lose the dead player's entity (ServerEntity.removePairing) and are sent the
    /// respawned one as a new entity at the next tick. Their clients would otherwise keep
    /// the dead body, lying down, wherever the player goes.
    pub fn respawned(&mut self, id: i32) {
        self.guests
            .update_all(|g| super::super::viewed_levels::entities::forget_entity(g, id));
    }

    /// A respawn point whose block is gone or blocked: the respawning player
    /// is sent NO_RESPAWN_BLOCK_AVAILABLE and says
    /// `block.minecraft.spawn.not_valid` in its chat.
    pub fn missing_respawn_block(&self, conn: u32) {
        self.send_to(
            conn,
            &C::GameEvent(play::GameEvent {
                event: game_event::NO_RESPAWN_BLOCK_AVAILABLE,
                param: 0.0,
            }),
        );
    }

    /// PlayerList.respawn: back to `at` with a fresh start. The vanilla client waits on its
    /// level loading screen from the Respawn packet until the level info's "level chunks load
    /// start" event and the chunk under the player, so the packets follow vanilla's order.
    /// `rotation` is the facing a mod's `player_respawn_position` chose; none
    /// is the level's respawn direction.
    pub fn begin_respawn(
        &mut self,
        i: usize,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        keep_attributes: bool,
        ctx: &TickCtx,
    ) {
        let info = self.spawn_info(i, 0, ctx.sea_level);
        self.close_container_for(i);
        self.guests.update(i, |g| {
            // A new player: a mod's collision exclusions were the dead one's (#3717).
            g.collision_exclusions.clear();
            // restartClientLoadTimerAfterRespawn: the client loads a whole new level before it
            // reports it loaded.
            g.load_timeout = super::super::guest::CLIENT_LOADED_TIMEOUT;
            g.loading = true;
            g.sent_health = None;
            g.sent_health_data = false;
            g.sent_pose = crate::pose::Pose::Standing;
            // ServerPlayer.restoreFrom: the game mode again (onUpdateAbilities).
            g.send(&g.abilities_packet());
            // The new player faces the level's respawn direction (snapTo), or
            // the one its TeleportTransition carries.
            [g.yaw, g.pitch] = rotation.unwrap_or(ctx.spawn_angle);
            g.place(at);
            g.send(&C::Respawn(play::Respawn {
                spawn: info,
                keep: i8::from(keep_attributes),
            }));
            g.sent_score = 0; // The replacement LocalPlayer starts with default entity data.
            // The client's new body needs the full replacement effect state.
            g.sent_effects.clear();
            g.center = None;
            g.view_dirty = false;
            g.sent_chunks.clear();
            g.chunk_queue.clear();
            g.tracker.clear();
            g.viewed.own.reset();
        });
        self.resend_teleport(i, None);
        let spawn = self.default_spawn(ctx.spawn, ctx.spawn_angle, ctx.spawn_dimension);
        self.guests.update(i, |g| {
            g.send(&spawn);
            g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
                difficulty: ctx.difficulty,
                locked: ctx.difficulty_locked,
            }));
            let xp = (g.xp_progress, g.xp_level, g.xp_total);
            g.send(&C::SetExperience(play::SetExperience {
                progress: xp.0,
                level: xp.1,
                total: xp.2,
            }));
            g.sent_xp = Some(xp);
            // ServerPlayer.sendPostEffects.
            let list = g.post_effects.clone();
            g.post_effects_dirty = false;
            g.send(&C::PostEffects(play::PostEffects { effects: list }));
        });
        self.send_active_effects(i);
        self.send_level_info(i, ctx, true, None);
        self.send_permission_level(i);
    }

    /// The rest of PlayerList.respawn, after sending the command tree.
    pub fn finish_respawn(&mut self, i: usize) {
        // ServerLevel.addRespawnedPlayer: the chunk tracking starts again at the player.
        self.guests.update(i, |g| {
            g.placed = true;
            g.respawning = false;
            g.placed_at = self.ticks;
            let (cx, cz) = g.chunk();
            g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: cx,
                z: cz,
            }));
            g.center = None;
            g.view_dirty = false;
            g.center_sent = Some((cx, cz));
            // ServerPlayer.initInventoryMenu.
        });
        self.send_inventory(i);
        // PlayerList.sendAllPlayerInfo: the replacement's held slot, which a respawn without
        // keep_inventory resets while the client still holds the old selection.
        let g = &self.guests[i];
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
    }

    /// The level the players are in: the server holds one at a time, and the
    /// hub's packets name it.
    pub fn set_dimension(&mut self, dimension: &str) {
        if self.config.dimension == dimension {
            return;
        }
        // What this level's players were told stays with it.
        let leaving = LevelSync {
            last_clock: self.last_clock,
            last_rate: self.last_rate,
            last_level_clock: self.last_level_clock,
            weather: self.weather,
        };
        let entering = self.level_sync.remove(dimension).unwrap_or_default();
        self.level_sync
            .insert(std::mem::take(&mut self.config.dimension), leaving);
        (
            self.last_clock,
            self.last_rate,
            self.last_level_clock,
            self.weather,
        ) = (
            entering.last_clock,
            entering.last_rate,
            entering.last_level_clock,
            entering.weather,
        );
        self.config.dimension = dimension.to_owned();
        self.dimension_type = self
            .sync
            .id(
                "minecraft:dimension_type",
                self.config.levels.dimension_type(dimension),
            )
            .unwrap_or(0);
        self.encoder
            .set_dimension(self.config.levels.get(dimension));
    }

    /// ServerPlayer.teleport(TeleportTransition) into the level the hub now names
    /// (`set_dimension`): ClientboundRespawnPacket keeping all data, then vanilla's packet order.
    /// Nothing tracked in the old level is removed one by one: the client drops its whole level.
    ///
    /// Through a `portal`, vanilla finds the exit (an End platform, a Nether
    /// portal, a spawn column) before the teleport; here it is found once
    /// the destination has loaded around `at`. Until [`Self::land`] the
    /// client gets no position, so it stays on its level loading screen
    /// instead of showing the player in the air at `at`.
    pub fn begin_level_change(
        &mut self,
        i: usize,
        at: [f64; 3],
        yaw: f32,
        pitch: Option<f32>,
        portal_cooldown: i32,
        ctx: &TickCtx,
    ) -> LevelChange {
        let info = self.spawn_info(i, portal_cooldown, ctx.sea_level);
        self.close_container_for(i);
        let view = self.before_travel(i);
        let g = &self.guests[i];
        g.send(&C::Respawn(play::Respawn {
            spawn: info,
            // ClientboundRespawnPacket.KEEP_ALL_DATA.
            keep: 3,
        }));
        if let Some(level @ sunrise_protocol::viewed_levels::Level::Bound { .. }) = view {
            g.sender.send(sunrise_protocol::viewed_levels::swap(level));
        }
        g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        self.send_permission_level(i);
        LevelChange {
            at,
            yaw,
            pitch,
            view,
        }
    }

    /// Finish the move after sending the command tree from the player's source state.
    pub fn finish_level_change(
        &mut self,
        i: usize,
        change: LevelChange,
        portal: bool,
        ctx: &TickCtx,
    ) {
        let LevelChange {
            at,
            yaw,
            pitch,
            view,
        } = change;
        let left = self.guests[i].dimension.clone();
        self.guests.update(i, |g| {
            // Out of the old level with its chunks (ChunkMap.removeEntity).
            super::super::chunk_watch::unwatch_all(&mut self.chunk_watches, g);
            g.dimension = self.config.dimension.clone();
            g.level_entered = self.ticks;
            // A mod's collision exclusions were the old level's (#3717).
            g.collision_exclusions.clear();
            // The load timer is not restarted: vanilla resets the client's loaded state on death and
            // respawn only, and covers a level change with its separate `isChangingDimension` term.
            g.center = None;
            g.view_dirty = false;
            g.yaw = yaw;
            if let Some(pitch) = pitch {
                g.pitch = pitch;
            }
            g.place(at);
        });
        // Its chunks, queues and entities left behind, or kept as a viewed level (#3716).
        let viewed_weather = self.leave_level(i, &left, view);
        if portal {
            // A teleport still pending from the old level is void: its late
            // acceptance must not move the player back there.
            self.guests.update(i, |g| {
                g.landing = true;
                g.awaiting_teleport = None;
                g.pending_travel_velocity = None;
                g.teleport_counter = (g.teleport_counter + 1) & 0x7FFF_FFFF;
            });
        } else {
            self.guests.update(i, |g| g.first_good = at);
            self.resend_teleport(i, None);
        }
        let g = &self.guests[i];
        g.send(&g.abilities_packet());
        self.send_level_info(i, ctx, !portal, viewed_weather);
        // PlayerList.sendAllPlayerInfo: the inventory and the held slot.
        self.send_inventory(i);
        let g = &self.guests[i];
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        self.send_active_effects(i);
        self.guests.update(i, |g| {
            // ServerPlayer.sendPostEffects.
            let list = g.post_effects.clone();
            g.post_effects_dirty = false;
            g.send(&C::PostEffects(play::PostEffects { effects: list }));
            // lastSentExp, lastSentHealth and lastSentFood; the client's new
            // player starts with the default air and no absorption.
            g.sent_health = None;
            g.sent_health_data = false;
            g.sent_xp = None;
            g.sent_air = (300, 0.0);
            g.sent_frozen = 0;
            // Respawn KEEP_ALL_DATA copies the old player's tracked values,
            // including shared flags. Preserve the publication baseline so a
            // flag cleared after the level change is still sent to that client.
            g.sent_shoulders = [None; 2];
            g.sent_stuck = (0, 0);
            let (cx, cz) = g.chunk();
            g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: cx,
                z: cz,
            }));
            g.center_sent = Some((cx, cz));
        });
    }

    /// The rest of a portal's [`Self::finish_level_change`] once its exit is found:
    /// the position, LEVEL_CHUNKS_LOAD_START (the client's loading screen
    /// closes once its chunk is in) and TeleportTransition.PLAY_PORTAL_SOUND.
    pub fn land(&mut self, i: usize, at: [f64; 3]) {
        {
            let value = false;
            self.guests.update(i, |guest| guest.landing = value);
        }
        self.guests.update(i, |g| g.place(at)); // hold the exit chunk
        self.teleport(i, Some(at));
        let g = &self.guests[i];
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        }));
        g.send(&C::LevelEvent(play::LevelEvent {
            kind: 1032,
            pos: [0, 0, 0],
            data: 0,
            global: false,
        }));
    }
}
