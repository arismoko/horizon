//! Boats and minecarts for a server's players: LAN guests, players on a
//! dedicated server, and single player's window on its server thread.
//!
//! A player aims at a vehicle, item frame, armor stand or painting as at a
//! mob (`ServerGamePacketListenerImpl.handleInteract`), and the game's own
//! code for it runs as that player (`AbstractBoat.interact` seats it,
//! `VehicleEntity.hurtServer` breaks the vehicle). A boat is driven by its
//! rider's client (`handleMoveVehicle`, `handlePaddleBoat`); a minecart moves
//! here with the rider's input (`ServerPlayer.getLastClientMoveIntent`).
//! Sneaking gets out (`Player.rideTick`: `wantsToStopRiding`), and the client
//! is teleported where the player lands (`ServerPlayer.dismountTo`).
use super::{Dim, Keys, lan::LanHost, minecarts::SEAT};
use sunrise_core::geometry::{Axis, Vec3};
use sunrise_level::collisions::SceneCollisions;
use sunrise_protocol::{
    packets::play::{self, Clientbound as C},
    text::translatable,
};
use sunrise_sim::lan::hub::no_blocks_in;
use sunrise_sim::mobs::equine;
use sunrise_world::entity_move::MoverType;

/// `PlayerInput` flags, as the server player's client last sent them.
pub fn input_keys(flags: u8) -> Keys {
    Keys {
        forward: flags & 1 != 0,
        backward: flags & 2 != 0,
        left: flags & 4 != 0,
        right: flags & 8 != 0,
        jump: flags & 16 != 0,
        shift: flags & 32 != 0,
        sprint: flags & 64 != 0,
        ..Keys::default()
    }
}

/// A boat or minecart a server player rode, followed for
/// `native_vehicle_moved`.
pub(in crate::game) struct WatchedVehicle {
    pub(in crate::game) id: u32,
    pub(in crate::game) dim: Dim,
    /// The server player it carries now.
    pub(in crate::game) rider: Option<u128>,
    /// Where the hook last heard of it.
    pub(in crate::game) at: [f64; 3],
    /// Ticks it has stood still.
    pub(in crate::game) still: u32,
}

impl crate::game::ServerGame<'_> {
    /// Where the boat or minecart `id` of this level is.
    pub(in crate::game::server_side) fn watched_vehicle_position(
        &self,
        id: u32,
    ) -> Option<[f64; 3]> {
        let at = |v: Vec3| [v.x, v.y, v.z];
        self.common
            .boats
            .iter()
            .find(|b| b.id == id)
            .map(|b| at(b.position))
            .or_else(|| {
                self.common
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| at(c.position))
            })
    }

    /// `VehicleMoveEvent` for the boats and minecarts server players ride
    /// (and left less than ten seconds ago): once a tick, after they moved.
    pub(in crate::game::server_side) fn tick_vehicle_watch(&mut self) {
        let riders: Vec<(u32, u32, u128)> = self.server.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter_map(|g| Some((g.vehicle?, g.conn, g.uuid)))
                .collect()
        });
        if riders.is_empty() && self.server.watched_vehicles.is_empty() {
            return;
        }
        let dim = self.common.dim;
        for &(id, _, uuid) in &riders {
            let Some(at) = self.watched_vehicle_position(id) else {
                continue;
            };
            match self.server.watched_vehicles.iter_mut().find(|w| w.id == id) {
                Some(w) => (w.dim, w.rider) = (dim, Some(uuid)),
                None => self.server.watched_vehicles.push(WatchedVehicle {
                    id,
                    dim,
                    rider: Some(uuid),
                    at,
                    still: 0,
                }),
            }
        }
        let mut kept = Vec::new();
        // (rider's connection and UUID, vehicle type, from, to)
        let mut moved = Vec::new();
        for mut w in std::mem::take(&mut self.server.watched_vehicles) {
            // Another level's vehicle is told of on that level's tick.
            if w.dim != dim {
                kept.push(w);
                continue;
            }
            // Broken, or gone from this level.
            let Some(now) = self.watched_vehicle_position(w.id) else {
                continue;
            };
            let rider = riders.iter().find(|r| r.0 == w.id);
            w.rider = rider.map(|r| r.2);
            if now != w.at {
                let from = std::mem::replace(&mut w.at, now);
                w.still = 0;
                let kind = self.target_type(
                    self.level_entity(w.id as i32)
                        .unwrap_or(super::commands::Target::Boat(w.id)),
                );
                moved.push((rider.map(|r| (r.1, r.2)), kind, from, now));
            } else {
                w.still += 1;
                if w.rider.is_none() && w.still > 200 {
                    continue;
                }
            }
            kept.push(w);
        }
        self.server.watched_vehicles = kept;
        // The rider's state is swapped in for the call, so the helpers reach
        // them; a vehicle nobody rides is told of with nobody swapped in.
        for (rider, kind, from, to) in moved {
            let Some((conn, uuid)) = rider else {
                self.native_vehicle_moved(None, &kind, from, to);
                continue;
            };
            let hook = |game: &mut Self| game.native_vehicle_moved(Some(uuid), &kind, from, to);
            if self.server.lan_acting == Some(conn) {
                hook(self);
            } else {
                self.as_guest(conn, hook);
            }
        }
    }

    /// The vehicles the server's players ride: they move with their riders,
    /// not in `tick_boats` and `tick_carts`.
    pub(in crate::game) fn guest_vehicles(&self) -> Vec<u32> {
        self.server.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub.guests.iter().filter_map(|g| g.vehicle).collect()
        })
    }

    /// Whether a server player other than the one acting rides `vehicle`
    /// (`Entity.canAddPassenger`: a boat or minecart seats one player here).
    pub(in crate::game) fn ridden_by_guest(&self, vehicle: u32) -> bool {
        self.server.lan.as_ref().is_some_and(|h| {
            h.hub
                .guests
                .iter()
                .any(|g| Some(g.conn) != self.server.lan_acting && g.vehicle == Some(vehicle))
        })
    }

    /// `Player.rideTick` for a server player, before its moves this tick:
    /// it stays in its seat, steers its minecart, or gets out.
    pub fn guest_vehicle_tick(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        let Some(vehicle) = g.vehicle.filter(|_| host.hub.here(g)) else {
            return;
        };
        if self.common.persist.removed_vehicles.contains_key(&vehicle) {
            // A canceled destruction dismount still owns the removed object.
            // Retry in the player's tick, not by silently dropping the edge.
            self.as_guest(conn, |game| game.stop_riding());
            return;
        }
        let (keys, dead) = (input_keys(g.input), g.dead || g.health <= 0.0);
        let boat = self.common.boats.iter().position(|b| b.id == vehicle);
        let cart = self.common.carts.iter().any(|c| c.id == vehicle);
        let mount = self.common.mobs.iter().any(|m| m.id == vehicle);
        let cushion = self.common.cushions.iter().any(|c| c.id == vehicle);
        let seat = self.seat(vehicle).is_some();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        // The vehicle is gone (broken, in another level) or the rider died:
        // its client drops it with the entity or the death screen.
        if dead || (boat.is_none() && !cart && !mount && !cushion && !seat) {
            host.hub.guests.update(gi, |g| {
                (g.vehicle, g.vehicle_good) = (None, None);
            });
            return;
        }
        let mut ejected = false;
        if let Some(i) = boat {
            // `Entity.ejectPassengers`: every player in the boat must see
            // the request; the common boat tick clears it once nobody rides.
            ejected = self.common.boats[i].eject_riders;
        }
        // Getting out goes through the put-back at the end of `guest_turn`, which tells the client and
        // teleports it (ServerPlayer.dismountTo).
        let seat = self.as_guest(conn, |game| {
            // A cushion does not move: the rider sits on it until it sneaks
            // (`Cushion.removePassenger` plays the get-up sound).
            if game.cushions.iter().any(|c| Some(c.id) == game.riding) {
                if keys.shift {
                    game.tick_cushion_riding(keys);
                    return None;
                }
                let c = game.cushions.iter().find(|c| Some(c.id) == game.riding)?;
                let p = c.seat();
                return Some([p.x, p.y, p.z]);
            }
            // An armor stand, a display, a marker...: the rider keeps its seat as the vehicle is
            // moved (/tp, a mod) until it sneaks.
            if game.riding.is_some_and(|id| game.seat(id).is_some()) {
                let p = game.tick_seat_riding(keys)?;
                return Some([p.x, p.y, p.z]);
            }
            if let Some(i) = game.boats.iter().position(|b| Some(b.id) == game.riding) {
                if keys.shift || ejected {
                    game.dismount(i);
                    return None;
                }
                let p = game.player_boat_seat(i);
                // AbstractBoat.positionRider's clampRotation sets the raw body yaw.
                if let Some(player) = &mut game.common.player {
                    player.body_yaw = game.common.boats[i].yaw;
                }
                return Some([p.x, p.y, p.z]);
            }
            // The minecart moves with the rider's input, and lets it out when
            // it sneaks or the cart breaks. An animal takes the rider's input
            // for its own tick (`travelRidden`), or lets it off.
            if !game.tick_cart_riding(keys) && game.tick_mount_riding(keys) {
                game.riding?;
                game.position_rider();
            }
            game.riding?;
            let p = game.player.as_ref()?.position;
            Some([p.x, p.y, p.z])
        });
        if let Some(Some(at)) = seat
            && let Some(host) = self.server.lan.as_mut()
        {
            host.hub.guests.update(gi, |g| {
                g.previous_position = g.position;
                g.position = at;
            });
        }
    }

    /// ServerGamePacketListenerImpl.tickPlayer records the vehicle's last good
    /// position after doTick, so movement by the server is not held against the
    /// rider's next packet.
    pub(in crate::game::server_side) fn guest_vehicle_movement_baseline(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let Some(vehicle) = guest.vehicle.filter(|_| host.hub.here(guest)) else {
            return;
        };
        let at = self
            .common
            .boats
            .iter()
            .find(|b| b.id == vehicle)
            .map(|b| b.position)
            .or_else(|| {
                self.common
                    .mobs
                    .iter()
                    .find(|m| m.id == vehicle && equine::moved_by_client(m))
                    .map(|m| m.position)
            });
        if let Some(p) = at {
            host.hub
                .guests
                .update(gi, |g| g.vehicle_good = Some([p.x, p.y, p.z]));
        }
    }

    /// After an interaction: a player that got into a vehicle sits in it,
    /// and its last good vehicle position is where the vehicle is.
    pub fn guest_boarded(&mut self, conn: u32, before: Option<u32>) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some(vehicle) = host.hub.guests[gi].vehicle else {
            return;
        };
        if before == Some(vehicle) {
            return;
        }
        // The host in the window rides it already.
        if self.common.riding == Some(vehicle) {
            {
                let value = before;
                host.hub.guests.update(gi, |guest| guest.vehicle = value);
            }
            return;
        }
        let at = self
            .common
            .boats
            .iter()
            .position(|b| b.id == vehicle)
            .map(|i| (self.common.boats[i].position, self.player_boat_seat(i)))
            .or_else(|| {
                self.common
                    .carts
                    .iter()
                    .find(|c| c.id == vehicle)
                    .map(|c| (c.position, c.position.add_vector(Vec3::new(0.0, SEAT, 0.0))))
            })
            .or_else(|| {
                self.common
                    .cushions
                    .iter()
                    .find(|c| c.id == vehicle)
                    .map(|c| (c.position, c.seat()))
            })
            .or_else(|| self.seat(vehicle).map(|s| (s.position, s.player_seat())));
        if let Some((p, seat)) = at {
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.hub.guests.update(gi, |g| {
                g.vehicle_good = Some([p.x, p.y, p.z]);
                g.previous_position = g.position;
                g.position = [seat.x, seat.y, seat.z];
                g.on_ground = false;
            });
            return;
        }
        // An animal: the rider sits where `positionRider` puts it.
        let seat = self.as_guest(conn, |game| {
            game.position_rider();
            game.player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z])
        });
        if let Some(Some(at)) = seat
            && let Some(host) = self.server.lan.as_mut()
        {
            host.hub.guests.update(gi, |g| {
                g.previous_position = g.position;
                g.position = at;
                g.on_ground = false;
            });
        }
    }

    /// `ServerGamePacketListenerImpl.handleMoveVehicle`: the boat or the
    /// saddled horse its rider drives goes where the client says, unless it
    /// moved too quickly or wrongly; then it stays, and the client is told
    /// where it is.
    pub fn guest_move_vehicle(&mut self, conn: u32, p: &play::MoveVehicleRequest) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let [x, y, z] = p.position;
        if !x.is_finite()
            || !y.is_finite()
            || !z.is_finite()
            || !p.yaw.is_finite()
            || !p.pitch.is_finite()
        {
            host.hub.kick(
                conn,
                translatable("multiplayer.disconnect.invalid_vehicle_movement", &[]),
            );
            return;
        }
        // Unlike MovePlayer, this checks for a resend even before the client has loaded,
        // and does not apply the packet's rotation while awaiting the teleport.
        if host.hub.update_awaiting_teleport(gi) || !host.hub.guests[gi].client_loaded() {
            return;
        }
        let g = &host.hub.guests[gi];
        let owner = host.hub.singleplayer_owner == Some(g.uuid);
        let allow_flight = host.hub.config.allow_flight;
        let (vehicle, good) = (g.vehicle, g.vehicle_good);
        // Only the vehicle's controlling passenger moves it: the move of a
        // player in a boat's second seat is ignored (#11541).
        if vehicle.is_some_and(|v| !self.guest_controls_vehicle(conn, v)) {
            return;
        }
        let back = |host: &mut LanHost, position: Vec3, yaw: f32| {
            let packet = play::MoveVehicle {
                position: [position.x, position.y, position.z],
                yaw,
                pitch: 0.0,
            };
            host.hub.send_to(conn, &C::MoveVehicle(packet));
        };
        // Only a boat, and a horse its rider's client moves, are driven by
        // their rider (getControllingPassenger); a minecart that is elsewhere
        // on the client is put back.
        let boat = vehicle.and_then(|v| self.common.boats.iter().position(|b| b.id == v));
        let mount = vehicle.and_then(|v| {
            self.common
                .mobs
                .iter()
                .position(|m| m.id == v && equine::moved_by_client(m))
        });
        // The vehicle's id, kind, position, rotation, motion, box, step height
        // and ground.
        let (id, old, yaw, velocity, bounds, step, on_ground) = if let Some(i) = boat {
            let b = &self.common.boats[i];
            (
                b.id,
                b.position,
                b.yaw,
                b.velocity,
                b.bounds(),
                0.0,
                b.ground.on_ground,
            )
        } else if let Some(i) = mount {
            let m = &self.common.mobs[i];
            let step = m.kind.step_height as f32;
            (
                m.id,
                m.position,
                m.yaw,
                m.velocity,
                m.bounds(),
                step,
                m.ground.on_ground,
            )
        } else {
            if let Some(c) = vehicle.and_then(|v| self.common.carts.iter().find(|c| c.id == v))
                && c.position.distance_squared(Vec3::new(x, y, z)) > 1.0
            {
                back(self.server.lan.as_mut().unwrap(), c.position, c.yaw);
            }
            return;
        };
        let good = good.map_or(old, |g| Vec3::new(g[0], g[1], g[2]));
        let target = Vec3::new(
            x.clamp(-3.0E7, 3.0E7),
            y.clamp(-2.0E7, 2.0E7),
            z.clamp(-3.0E7, 3.0E7),
        );
        // Moved too quickly: more than 10 blocks past its own motion.
        let moved = target.distance_squared(old) - velocity.length_squared();
        if moved > 100.0 && !owner {
            log::warn!("vehicle {id} moved too quickly! {moved}");
            back(self.server.lan.as_mut().unwrap(), old, yaw);
            return;
        }
        // A living mount on a climbable has no fall, before its move: a move
        // put back below keeps that.
        if let Some(i) = mount
            && self.common.mobs[i].on_climbable(&self.mob_world_base(&[]))
        {
            self.common.mobs[i].fall_distance = 0.0;
        }
        // vehicle.move(PLAYER, target - lastGood) with the vehicle's
        // collisions: where it can't follow the client (a quarter block off,
        // the height aside), it moved wrongly. A Player-cause move through
        // the move gate (#2414); PR 1a records the movement unchanged.
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        let wanted = target.subtract(good);
        // The vehicle's type name only if a listener asks for it (#2414: no
        // string on a move nobody listens to).
        let kind = || match (boat, mount) {
            (Some(i), _) => std::borrow::Cow::Owned(self.common.boats[i].entity_type()),
            (None, Some(i)) => std::borrow::Cow::Borrowed(&*self.common.mobs[i].kind.id),
            // Unreachable: the vehicle was a boat or a mount, or it returned.
            (None, None) => std::borrow::Cow::Borrowed(""),
        };
        let mut gate = sunrise_sim::move_gate::armed_with_controlled(
            &self.common.loop_context,
            id,
            kind,
            MoverType::Player,
            wanted,
            true,
        );
        let wanted = gate.delta();
        // A listener's decision that is not the request (#2414, ruling 5):
        // the driver is put back whatever the distance - the flag comes from
        // the gate, not from the numbers.
        let overridden = gate.overridden();
        let resolved = sunrise_sim::move_gate::scene_resolve(
            &mut gate, &mut scene, bounds, wanted, step, on_ground,
        )
        .unwrap_or(wanted);
        let reached = old.add_vector(resolved);
        let wrongly =
            overridden || (target.x - reached.x).powi(2) + (target.z - reached.z).powi(2) > 0.0625;
        let free_before = scene.unobstructed(bounds).unwrap_or(true);
        // isEntityCollidingWithAnythingNew: a box that was free is not there.
        let moved_box = bounds.move_by(target.subtract(old));
        let free_after = scene
            .unobstructed(moved_box.deflate(Vec3::new(1.0E-5, 1.0E-5, 1.0E-5)))
            .unwrap_or(true);
        // Entity.move in `vehicle.move`, before the checks: a long move through
        // water or a reset block resets the fall, which a move put back keeps
        // (`absSnapTo` restores only the position).
        if let Some(i) = mount {
            self.common.mobs[i].fall_distance =
                self.common
                    .vehicle_fall_reset(self.common.mobs[i].fall_distance, old, resolved);
        } else if let Some(i) = boat {
            let reset =
                self.common
                    .vehicle_fall_reset(self.common.boats[i].fall_distance, old, resolved);
            self.common
                .boats
                .update(i, |boat| boat.fall_distance = reset);
        }
        // An overridden decision is put back from wherever the vehicle starts:
        // the free-old-box exemption is the distance test's (review round 1),
        // like the hub's unobstructed-start check.
        if overridden || (free_before && (wrongly || !free_after)) {
            log::warn!("vehicle {id} moved wrongly");
            back(self.server.lan.as_mut().unwrap(), old, yaw);
            return;
        }
        // clientVehicleIsFloating: the move didn't go down a step, the vehicle hadn't come down onto
        // something on its last move (`verticalCollisionBelow` before this one), and nothing but air is
        // around its box grown by 0.0625 and stretched 0.55 down (water holds a boat up). A flying or
        // no-gravity mount, and a server that allows flight, never float.
        let (gravity, no_gravity) = match mount {
            Some(i) => (0.08, self.common.mobs[i].no_gravity),
            None => (0.04, false),
        };
        let around = moved_box
            .inflate(Vec3::new(0.0625, 0.0625, 0.0625))
            .expand_towards(Vec3::new(0.0, -0.55, 0.0));
        let corner = |max: bool| {
            [Axis::X, Axis::Y, Axis::Z].map(|a| if max { around.max(a) } else { around.min(a) })
        };
        let no_blocks = no_blocks_in(
            &self.common.world,
            corner(false),
            corner(true),
            &self.server.lan.as_ref().unwrap().hub.encoder,
        );
        self.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(gi, |g| {
                // `ServerChunkCache.move` after `absSnapTo` of the accepted move.
                g.release_tickets();
                g.vehicle_good = Some([target.x, target.y, target.z]);
                let rested = std::mem::replace(
                    &mut g.vehicle_rests,
                    wanted.y < 0.0 && resolved.y != wanted.y,
                );
                g.vehicle_floating =
                    wanted.y >= -0.03125 && !rested && !allow_flight && !no_gravity && no_blocks;
                g.vehicle_gravity = gravity;
            });
        if let Some(i) = mount {
            use sunrise_sim::mobs::living::MovementAction;

            let (id, join) = (self.common.mobs[i].id, self.common.mobs[i].join);
            let mut events = Vec::new();
            let mut resume = None;
            while let Some(i) = self.common.mob_index.find(&self.common.mobs, id) {
                if self.common.mobs[i].join != join || self.common.mobs[i].removed {
                    break;
                }
                let mut m = self.common.mobs[i].clone();
                let mut lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
                let mut paused_hurt = None;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let w = self.mob_world_base(&[]);
                    let mut level = sunrise_sim::health::DamageLevel::lent(
                        &self.common.world,
                        self.common.pack.assets.registry,
                        &mut lent,
                        Some(self.items()),
                        Some(&self.common.pack.block_registries),
                        &self.common.damage_hooks,
                    )
                    .with_variants(self.world_variants())
                    .with_reads(crate::game::entity_server::hit_reads_of!(self))
                    .with_mob_hurt_handoff(&mut paused_hurt);
                    if let Some(pause) = resume.take() {
                        m.resume_client_moved_fall(&w, &mut level, &mut events, pause)
                    } else {
                        // absSnapTo, then setOnGroundWithMovement and doCheckFallDamage
                        // with the client's ground and move; the rider's tick seats it.
                        let delta = target.subtract(m.position);
                        m.position = target;
                        m.yaw = p.yaw;
                        m.head_pitch = p.pitch;
                        m.client_moved_fall(&w, &mut level, delta, p.on_ground, &mut events)
                    }
                }));
                lent.give_back(&mut self.common.world);
                let pause = outcome.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                self.common.mobs[i] = m;
                self.server.paused_mob_hurt.0 = paused_hurt;
                // LivingEntity.checkFallDamage sends dust before Block.fallOn.
                // Send it to every nearby player, including the driver.
                for event in events.extract_if(.., |e| {
                    matches!(e, sunrise_sim::mobs::MobEvent::LandingDust { .. })
                }) {
                    if let sunrise_sim::mobs::MobEvent::LandingDust {
                        state,
                        at,
                        count,
                        speed,
                    } = event
                    {
                        self.server_landing_dust(
                            state,
                            Vec3::new(at[0], at[1], at[2]),
                            count,
                            speed,
                        );
                    }
                }
                let Some(pause) = pause else {
                    break;
                };
                match pause.action() {
                    MovementAction::Trample(trample) => self.trample_farmland(id, false, trample),
                    MovementAction::SettleHit => {
                        self.finish_mob_damage_at(i);
                        self.settle_creaking_hurt(id);
                    }
                    MovementAction::Passengers(fall) => {
                        self.fall_to_passengers(id, fall.distance, fall.modifier);
                    }
                }
                resume = Some(pause);
            }
            let _ = self.handle_mob_events(events);
            return;
        }
        let Some(i) = boat else {
            return;
        };
        self.common.boats.update(i, |b| {
            b.previous = old;
            b.velocity = target.subtract(old);
            b.position = target;
            b.previous_yaw = b.yaw;
            b.yaw = p.yaw;
            b.ground.on_ground = p.on_ground;
        });
        // handleMoveVehicle's `vehicle.move` counts the fall
        // (AbstractBoat.checkFallDamage); the server's baseTick resets it in
        // water (updateFluidInteraction), which this boat's skipped tick
        // doesn't run.
        use super::entity::Entity as _;
        let mut probe = self.common.boats[i].clone();
        let in_water = probe
            .push_by_fluid(
                &self.common.world,
                &self.common.res.motion().fluids,
                &self.common.loop_context,
            )
            .0
            > 0.0;
        // The long move reset ran before collision validation.
        let fall = if in_water {
            0.0
        } else {
            self.common.boat_fall(&self.common.boats[i], resolved.y)
        };
        self.common.boats.update(i, |b| b.fall_distance = fall);
        let seat = self.player_boat_seat(i);
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        host.hub.guests.update(gi, |g| {
            g.previous_position = g.position;
            g.position = [seat.x, seat.y, seat.z];
        });
    }

    /// `Entity.getControllingPassenger` of a vehicle a server player rides: a
    /// boat has two seats, and only its first player passenger drives it. The
    /// window player sits first when it rides; of the server's players it is
    /// the one that got in first.
    pub(in crate::game::server_side) fn guest_controls_vehicle(
        &self,
        conn: u32,
        vehicle: u32,
    ) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        if self.common.riding == Some(vehicle) {
            return false;
        }
        host.hub
            .guests
            .iter()
            .filter(|g| g.vehicle == Some(vehicle))
            .min_by_key(|g| g.vehicle_order)
            .is_some_and(|g| g.conn == conn)
    }

    /// `handlePaddleBoat`: the paddles of the boat the player controls
    /// (`player.getControlledVehicle()`); a passenger in the second seat moves
    /// no paddle.
    pub(in crate::game::server_side) fn guest_paddle_boat(
        &mut self,
        conn: u32,
        paddles: [bool; 2],
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let vehicle = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .and_then(|g| g.vehicle);
        let Some(vehicle) = vehicle.filter(|&v| self.guest_controls_vehicle(conn, v)) else {
            return;
        };
        if let Some(i) = self.common.boats.iter().position(|b| b.id == vehicle) {
            self.common.boats.update(i, |b| {
                b.paddling = paddles;
                b.paddle_controller = true;
            });
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::mounts_common::MountsCommonGameExt1 as _;
