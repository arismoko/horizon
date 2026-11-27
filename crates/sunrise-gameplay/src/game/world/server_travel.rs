//! A server's players between dimensions.
//!
//! A server holds a level per dimension (game/world/levels.rs). A player who steps
//! into a Nether or End portal moves to the destination's level, which opens
//! if nobody was there: the portal plans and arrival search pick the exit,
//! and the player hears it as `ServerPlayer.teleport` into
//! another level: a Respawn packet keeping all data, then a teleport to the
//! exit once the destination has loaded around them.

use super::nether_portal::Plan;
use super::respawn_anchor::RespawnConfig;
use super::respawn_position::Respawn;
use super::{Dim, Mob, lan::LanHost, minecarts};
use std::ops::ControlFlow;
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_gameplay_common::game::end_state::GatewayJob;
use sunrise_gameplay_common::game::entity_memory::OffLevel;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::nether_portal::PLAYER_DELAY;
use sunrise_sim::{lan::hub::TickCtx, world_spawn::PlayerSpawnFinder};

/// Why an entity changes level (`entity_travel_to_dimension`'s `cause`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TravelCause {
    NetherPortal,
    EndPortal,
    EndGateway,
    /// A teleport into another level (`/tp`, a spectator's teleport).
    Teleport,
}

impl TravelCause {
    /// The name a mod is told.
    pub(in crate::game::server_side) fn id(self) -> &'static str {
        match self {
            Self::NetherPortal => "nether_portal",
            Self::EndPortal => "end_portal",
            Self::EndGateway => "end_gateway",
            Self::Teleport => "teleport",
        }
    }

    /// The portal a plan goes through.
    pub(in crate::game::server_side) fn of_plan(plan: &Plan) -> Self {
        if plan.end {
            Self::EndPortal
        } else {
            Self::NetherPortal
        }
    }
}

/// A trip under way: the player is in the destination's level, whose
/// chunks are loading around `target`.
pub struct Trip {
    pub conn: u32,
    pub dim: Dim,
    /// The level the player came from: the vehicle it rides waits there
    /// until the exit is found.
    pub source: Dim,
    pub target: Vec3,
    /// Through an End portal.
    end: bool,
    /// Into the End through its portal, building its platform
    /// (`EndPortalBlock.getPortalDestination` from another level); else an
    /// exit.
    entering: bool,
    /// The Nether portal plan, whose exit is found or built on arrival.
    plan: Option<Plan>,
    /// The vehicle the player rides, which travels with it
    /// (`Entity.teleportCrossDimension` carries the passengers).
    pub vehicle: Option<u32>,
    /// Through an End gateway, within the level: its exit, found or built
    /// on arrival.
    gateway: Option<GatewayJob>,
    /// A respawn at this point outside the Overworld, resolved through its
    /// block on arrival.
    pub respawn: Option<RespawnConfig>,
    /// A transition that resolves its spawn destination before moving the player.
    pub spawn_search: Option<SpawnTrip>,
    /// A respawn at the world spawn, moved onto a free column within the
    /// respawn radius once the chunks around it have loaded.
    pub world_spawn: bool,
    /// The respawn point whose bed or anchor was gone, for a world-spawn
    /// respawn that replaces it (`TeleportTransition.missingRespawnBlock`).
    missing: Option<RespawnConfig>,
    /// The search for that column while a candidate's chunk loads.
    pub finder: Option<PlayerSpawnFinder>,
    /// Sent by a command: the player is already where it goes.
    placed: bool,
    /// A command teleport to chunks not loaded yet: the client's moves are ignored until the
    /// destination is ready, whether placed by a level change or waiting for another position packet.
    hold: bool,
    /// The level the player left for another one: the mods are told they
    /// changed dimension when the trip ends (`tell_mods_changed_dimension`).
    pub(in crate::game) left: Option<Dim>,
}

#[derive(Clone, Copy)]
pub(in crate::game) enum SpawnAction {
    Replace { was_death: bool },
    EndExit,
}

pub struct SpawnTrip {
    action: SpawnAction,
    pub waiting_for: Option<(i32, i32)>,
    /// A saved personal column without a block respawn config. An End exit lands on
    /// its surface after it loads and retains the player's facing.
    legacy_personal_point: bool,
}

/// A vehicle a player rides: an animal, a boat or a minecart. It travels
/// with the player through a portal (`Entity.teleportCrossDimension` carries
/// the passengers, and they ride the vehicle that is recreated in the other
/// dimension).
trait Rideable {
    fn id(&self) -> u32;
    /// The vehicle stands where its rider arrives, with the motion it crossed
    /// with (none for the kinds that keep none).
    fn arrive(&mut self, at: Vec3, yaw: f32, motion: Vec3);
}

impl Rideable for Mob {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32, _motion: Vec3) {
        sunrise_sim::mobs::living::teleports::snap_to(self, at);
        self.velocity = Vec3::ZERO;
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.fall_distance = 0.0;
    }
}

impl Rideable for sunrise_gameplay_common::game::boats_common::Boat {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32, _motion: Vec3) {
        self.position = at;
        self.previous = at;
        self.velocity = Vec3::ZERO;
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.delta_rotation = 0.0;
    }
}

impl Rideable for minecarts::Minecart {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32, motion: Vec3) {
        self.position = at;
        self.previous = at;
        // `PositionMoveRotation.of(this)` takes the cart's `getKnownMovement`
        // (`OldMinecartBehavior` clamps it to 0.4 either way along x and z):
        // it rolls on at that speed instead of arriving at rest.
        self.velocity = Vec3::new(
            motion.x.clamp(-0.4, 0.4),
            motion.y,
            motion.z.clamp(-0.4, 0.4),
        );
        self.yaw = yaw;
        self.previous_yaw = yaw;
    }
}

/// The hub's view of the server for packets sent outside its own tick.
macro_rules! server_ctx {
    ($s:ident) => {{
        let spawn = $s.world_spawn_position();
        let clock_rate = $s.network_clock_rate();
        TickCtx {
            world: &mut $s.common.world,
            entities: &[],
            next_entity: &mut $s.server.next_entity,
            spawn,
            spawn_angle: $s.common.level.world_spawn_angle,
            spawn_dimension: sunrise_gameplay_common::game::Dim::of_world_spawn(&$s.common.level)
                .id(),
            sea_level: $s
                .common
                .end
                .locator()
                .map_or(63, |terrain| terrain.sea_level()),
            difficulty: i32::from($s.common.ui.difficulty),
            difficulty_locked: $s.common.level.difficulty_locked,
            clock_rate,
            tick_rate: $s.common.clock.rate(),
            tick_frozen: $s.common.clock.frozen(),
            tick_steps: $s.common.clock.step_state(),
            collision: Some($s.common.collision),
            border: &$s.common.border,
            streaming: Some(&$s.server.streaming),
        }
    }};
}

/// The vehicle `id` leaves the level it is in, at the point its rider
/// arrives. It keeps its entity id, so the ride is never broken.
/// A guest leaving for another level leaves its fishing hook behind in the
/// old one, where vanilla's hook stays until its distance check ends it. A
/// guest's hook lives on the guest here, so it ends now: it must not act on
/// the blocks of the level the guest arrives in.
fn drop_hook(host: &mut LanHost, conn: u32) {
    if let Some(play) = host.play.get_mut(&conn) {
        play.hook.discard();
    }
}

fn take_vehicle<
    T: Rideable
        + sunrise_sim::join_seq::Joined
        + sunrise_mod_runtime::attachments::HasAttachmentLifetime,
    const DIRECT: bool,
>(
    list: &mut sunrise_sim::join_seq::JoinList<T, DIRECT>,
    id: u32,
    at: Vec3,
    yaw: f32,
    motion: Vec3,
) -> Option<T> {
    let i = list.iter().position(|v| v.id() == id)?;
    let mut v = list.remove_for_transfer(i);
    v.arrive(at, yaw, motion);
    v.set_join_seq(sunrise_sim::join_seq::JoinSeq::UNJOINED);
    Some(v)
}

/// `TeleportCommand` into another level for a server's player: where, and
/// the rotation it sets (none keeps the player's).
pub struct LevelTeleport {
    pub conn: u32,
    pub dim: Dim,
    pub at: [f64; 3],
    pub rotation: Option<[f32; 2]>,
    /// `TeleportTransition.asPassenger`: the vehicle already moved with its tree.
    pub vehicle: Option<u32>,
}

#[derive(Clone, Copy)]
enum LevelTeleportKind {
    Direct,
    EndExit { missing: bool },
}

/// A teleport a command made inside a level: told to the mods once it has
/// reached the hub (`native_player_teleported`).
pub(in crate::game) struct TeleportNote {
    pub conn: u32,
    pub uuid: u128,
    pub name: String,
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub dimension: String,
}

/// A player's box: Player's standing dimensions.
const WIDTH: f64 = 0.6;
const HEIGHT: f64 = 1.8;

impl Trip {
    pub fn is(&self, conn: u32) -> bool {
        self.conn == conn
    }

    pub(in crate::game::server_side) fn spawn_destination_areas(
        &self,
        dim: Dim,
    ) -> impl Iterator<Item = ((i32, i32), i32)> {
        self.spawn_search
            .as_ref()
            .filter(|_| self.dim == dim)
            .into_iter()
            .flat_map(|search| {
                std::iter::once((crate::game::ServerGame::chunk_of(self.target), 2))
                    .chain(search.waiting_for.map(|chunk| (chunk, 0)))
            })
    }

    fn is_resolving_end_exit(&self) -> bool {
        self.spawn_search
            .as_ref()
            .is_some_and(|s| matches!(s.action, SpawnAction::EndExit))
    }

    /// Respawns and command arrivals hold their loading area; an unresolved End teleport
    /// must not change the source player's placement or movement before the travel veto.
    fn holds_movement(&self) -> bool {
        !self.is_resolving_end_exit() && (self.hold || self.respawn.is_some() || self.world_spawn)
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn gateway_source_chunks(&self) -> Vec<(i32, i32)> {
        self.server
            .server_trips
            .iter()
            .filter(|t| t.dim == self.common.dim)
            .filter_map(|t| t.gateway.and_then(GatewayJob::source_chunk))
            .collect()
    }
    /// A server player's `tp` inside their level, where the chunks round `target` have not loaded: vanilla loads
    /// them before the player arrives, so the player is held as a trip holds them (their body is left alone, their
    /// client's moves ignored) until the chunks are lit, then placed there again (#15727).
    pub fn hold_for_chunks(&mut self, conn: u32, target: Vec3) {
        // The arrival the player was waiting for elsewhere is abandoned.
        self.server.server_trips.retain(|t| !t.is(conn));
        let (cx, cz) = (
            (target.x.floor() as i32) >> 4,
            (target.z.floor() as i32) >> 4,
        );
        if (-2..=2).all(|dz| (-2..=2).all(|dx| self.server.streaming.is_lit(cx + dx, cz + dz))) {
            return;
        }
        self.server.server_trips.push(Trip {
            conn,
            dim: self.common.dim,
            source: self.common.dim,
            target,
            end: false,
            entering: false,
            plan: None,
            vehicle: None,
            gateway: None,
            respawn: None,
            spawn_search: None,
            world_spawn: false,
            finder: None,
            placed: false,
            hold: true,
            missing: None,
            left: None,
        });
    }
    /// A command hold follows the player where a mod sent them (`native_player_teleport` may
    /// cancel or redirect it). A respawn keeps its own destination and stand-up search.
    fn follow_command_arrivals(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let level = self.common.dim.id();
        let moved: Vec<(u32, Vec3)> = self
            .server
            .server_trips
            .iter()
            .filter(|t| t.hold && t.dim == self.common.dim)
            .filter_map(|t| {
                let g = host.hub.guests.iter().find(|g| g.conn == t.conn)?;
                let at = Vec3::new(g.position[0], g.position[1], g.position[2]);
                (g.dimension == level && at != t.target).then_some((t.conn, at))
            })
            .collect();
        for (conn, at) in moved {
            self.hold_for_chunks(conn, at);
        }
    }

    /// Release movement after completed or replaced arrivals. A spawn search can requeue
    /// itself while a candidate chunk loads: that still owns the hold until it finishes.
    fn release_held_arrivals(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        for i in 0..host.hub.guests.len() {
            let g = &host.hub.guests[i];
            if !g.tp_held {
                continue;
            }
            let conn = g.conn;
            if self
                .server
                .server_trips
                .iter()
                .any(|t| t.is(conn) && t.holds_movement())
            {
                continue;
            }
            let landing = self
                .server
                .server_trips
                .iter()
                .any(|t| t.is(conn) && (t.end || t.plan.is_some()));
            host.hub.guests.update(i, |g| {
                g.tp_held = false;
                if !landing {
                    g.landing = false;
                }
            });
        }
    }
    /// Whether the server leaves the player's body alone until arrival. A player whose
    /// End exit is still being resolved hasn't traveled yet and ticks in the source level.
    pub fn on_server_trip(&self, conn: u32) -> bool {
        self.server
            .server_trips
            .iter()
            .any(|t| t.is(conn) && !t.is_resolving_end_exit())
    }

    /// Portal processing does not start another trip while one is already being resolved.
    pub fn has_pending_travel(&self, conn: u32) -> bool {
        self.server.server_trips.iter().any(|t| t.is(conn))
    }

    /// Whether a vehicle waits in `dim` for its rider to arrive elsewhere:
    /// that level stays open until it has moved.
    pub fn vehicle_waits_in(&self, dim: Dim) -> bool {
        self.server
            .server_trips
            .iter()
            .any(|t| t.vehicle.is_some() && t.source == dim)
    }

    /// Per server tick in each level, after the guests' own: the arrivals of
    /// the trips into this level, then `Entity.handlePortal` for each player
    /// in it (Nether portals count up their transition time, End portals
    /// take them at once).
    pub fn tick_server_travel(&mut self) {
        if self.server.lan.is_none() {
            return;
        }
        // Outside a server only the respawns at the world spawn wait for chunks.
        self.arrive_server_trips();
        if self.common.options.dedicated.is_none() {
            return;
        }
        self.carry_riders_through_portals();
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.spectator())
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            if self.has_pending_travel(conn) {
                continue;
            }
            self.handle_server_portal(conn);
        }
    }

    /// `Entity.portalProcess` and `portalCooldown` of the player on `conn`: its own, kept in
    /// its record, where its turns (`as_guest`) and the mods called during its trip read them.
    #[doc(hidden)]
    pub fn guest_portal(
        &mut self,
        conn: u32,
    ) -> Option<&mut sunrise_sim::nether_portal::PortalState> {
        Some(
            &mut self
                .server
                .lan
                .as_mut()?
                .play
                .entry(conn)
                .or_default()
                .portal,
        )
    }

    /// The player's `portalCooldown`, 0 for one without a record.
    pub fn guest_portal_cooldown(&mut self, conn: u32) -> i32 {
        self.guest_portal(conn).map_or(0, |portal| portal.cooldown)
    }

    /// `Entity.setPortalCooldown`: `Player.getDimensionChangingDelay`.
    fn set_guest_portal_cooldown(&mut self, conn: u32) {
        if let Some(portal) = self.guest_portal(conn) {
            portal.cooldown = PLAYER_DELAY;
        }
    }

    /// `ServerPlayerGameMode.useItemOn` for a spectator: a `Portal` block
    /// (Nether portal, End portal, End gateway) takes the player to its
    /// destination (`getPortalDestination`, `Entity.teleportToPortalDestination`)
    /// after the camera goes back to the player, or says `spectator.cannot_teleport`
    /// when it has none. Returns whether the block was a portal (`CONSUME`).
    pub fn spectator_use_portal(&mut self, conn: u32, gi: usize, pos: [i32; 3]) -> bool {
        let name = self.block_name(pos);
        let name = name.trim_start_matches("minecraft:");
        if !matches!(name, "nether_portal" | "end_portal" | "end_gateway") {
            return false;
        }
        if self.has_pending_travel(conn) {
            return true;
        }
        let Some(g) = self.server.lan.as_ref().map(|host| &host.hub.guests[gi]) else {
            return false;
        };
        let at = Vec3::new(g.position[0], g.position[1], g.position[2]);
        let seen_credits = g.seen_credits;
        // EndGatewayBlock.getPortalDestination: none without its block entity.
        let gateway_ok = name != "end_gateway"
            || self
                .common
                .world
                .block_entities
                .get(&(pos[0], pos[1], pos[2]))
                .is_some_and(|e| {
                    matches!(e, sunrise_level::containers::BlockEntity::EndGateway(_))
                });
        if !gateway_ok {
            self.as_guest(conn, |g| {
                g.server_send_overlay_message(
                    sunrise_game_data::text::Text::translate("spectator.cannot_teleport", &[])
                        .with_color(sunrise_game_data::text::RED),
                );
            });
            return true;
        }
        if self.server.cmds.0.guest_cameras.contains_key(&conn) {
            self.guest_set_camera(gi, None);
        }
        match name {
            "nether_portal" => {
                let plan = self.portal_plan(
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    at,
                    WIDTH as f32,
                    HEIGHT as f32,
                    true,
                );
                // Entity.teleportToPortalDestination: isAllowedToEnterPortal.
                if self.may_enter(plan.dim) {
                    // NetherPortalBlock.getExitPortal: a spectator never
                    // builds an exit portal, it gets no transition without one.
                    if !self.spectator_exit_exists(&plan) {
                        self.as_guest(conn, |g| {
                            g.server_send_overlay_message(
                                sunrise_game_data::text::Text::translate(
                                    "spectator.cannot_teleport",
                                    &[],
                                )
                                .with_color(sunrise_game_data::text::RED),
                            );
                        });
                        return true;
                    }
                    if self.server_player_travel_allowed(conn, plan.dim, TravelCause::NetherPortal)
                    {
                        self.start_server_trip(conn, plan.dim, Some(plan));
                    }
                }
            }
            "end_portal" => {
                let from_end = self.common.dim == Dim::End;
                // Entity.canTeleport: out of the End only after the credits.
                if from_end {
                    if seen_credits {
                        self.queue_spawn_trip(conn, SpawnAction::EndExit);
                    }
                } else if self.server_player_travel_allowed(conn, Dim::End, TravelCause::EndPortal)
                {
                    self.start_server_trip(conn, Dim::End, None);
                }
            }
            _ => {
                if self.server_player_travel_allowed(conn, self.common.dim, TravelCause::EndGateway)
                {
                    self.start_gateway_trip(conn, pos);
                }
            }
        }
        true
    }

    /// Whether an exit portal already stands in the plan's dimension
    /// (`PortalForcer.findClosestPortalPosition`), searched without building.
    fn spectator_exit_exists(&mut self, plan: &Plan) -> bool {
        let source = self.common.dim;
        if self.enter_level(plan.dim).is_err() {
            return false;
        }
        let exists = self.find_exit_portal(plan).is_some();
        if let Err(e) = self.enter_level(source) {
            log::error!("could not reopen {}: {e}", source.id());
        }
        exists
    }

    /// Whether this guest's dimension change may start
    /// (`entity_may_travel`).
    pub fn server_player_travel_allowed(
        &mut self,
        conn: u32,
        target: Dim,
        cause: TravelCause,
    ) -> bool {
        let Some(g) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return false;
        };
        let entity = g.entity_id;
        self.entity_may_travel(entity as u32, target, cause)
    }

    /// `EntityTravelToDimensionEvent`: whether the entity `id` (the network
    /// id its handle names) may travel to the level `to`, asked before
    /// anything of it moves. Only a server asks, and only when a mod listens.
    pub fn entity_may_travel(&mut self, id: u32, to: Dim, cause: TravelCause) -> bool {
        self.entity_tree_may_travel(id, to, cause, &mut Vec::new())
    }

    fn entity_tree_may_travel(
        &mut self,
        id: u32,
        to: Dim,
        cause: TravelCause,
        visited: &mut Vec<u32>,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::EntityTravelToDimension)
        {
            return true;
        }
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return true;
        };
        if visited.contains(&id)
            || (!self.server.held_out.contains(&id)
                && self
                    .native_entity_target(mod_api::EntityRef::new(epoch, id))
                    .is_none())
        {
            return false;
        }
        let entity = mod_api::EntityRef::new(epoch, id);
        // A callback can reparent a participant into a later subtree.
        // Keep the whole transition's visited set, not only its active path.
        visited.push(id);
        let mut riders = Vec::new();
        let _ = self.entity_visit_passengers(entity, &mut |rider| {
            riders.push(rider);
            ControlFlow::Continue(())
        });
        let mut cancel = false;
        self.native_entity_travel_to_dimension(
            mod_api::EntityRef::new(epoch, id),
            to.id(),
            cause.id(),
            &mut cancel,
        );
        // A discard hides the handle immediately, even when physical removal
        // waits for the entity sweep. Never serialize that deferred copy.
        // Falling blocks tick held out of the level and cannot be addressed
        // through a handle during this callback.
        if cancel {
            return false;
        }
        if self.server.held_out.contains(&id) {
            return true;
        }
        if self
            .native_entity_target(mod_api::EntityRef::new(epoch, id))
            .is_none()
        {
            return false;
        }
        // One snapshot in the shared player/mob boarding order. Ejection can
        // itself call mods; refuse changed edges instead of overriding them.
        for &rider in riders.iter().rev() {
            if self.entity_vehicle(rider) != Some(entity) {
                continue;
            }
            if !self.entity_stop_riding(rider) && self.entity_vehicle(rider) == Some(entity) {
                return false;
            }
        }
        let accepted: Vec<_> = riders
            .into_iter()
            .filter(|rider| {
                self.entity_is_passenger(*rider) == Some(false)
                    && self.entity_tree_may_travel(rider.id(), to, cause, visited)
            })
            .collect();
        for rider in accepted {
            if self.entity_is_passenger(rider) == Some(false)
                && !self
                    .common
                    .mobs
                    .iter()
                    .any(|m| m.id == rider.id() && !m.alive())
            {
                // Reject callback topology changes: the shared mount path
                // checks live handles, dimension, cycles and vehicle capacity.
                let _ = self.entity_start_riding(rider, entity, true);
            }
        }
        self.native_entity_target(mod_api::EntityRef::new(epoch, id))
            .is_some()
    }

    /// `Entity.handlePortal` for one player, on its own portal state.
    pub fn handle_server_portal(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (at, invulnerable, sleeping, riding, seen_credits) = (
            Vec3::new(g.position[0], g.position[1], g.position[2]),
            g.creative(),
            g.sleeping,
            g.vehicle.is_some(),
            g.seen_credits,
        );
        // `LivingEntity.canUsePortal` (#943): alive, not a passenger — its vehicle
        // goes through and carries it (`carry_riders_through_portals`) — and
        // not asleep. `handlePortal` and `setAsInsidePortal` both ask it this tick.
        let usable = g
            .motion
            .as_deref()
            .is_some_and(|p| self.common.can_use_portal_of(p, riding, sleeping));
        // Entity.checkInsideBlocks: the Nether portal blocks the box is in.
        let inside = usable.then(|| self.nether_portal_inside(at)).flatten();
        let (end_portal, gateway) = self.inside_end_portals(at, WIDTH, HEIGHT);
        if end_portal && !riding {
            // EndPortalBlock.entityInside: the first exit from the End shows the credits.
            if self.common.dim == Dim::End && !seen_credits {
                self.remove_old_player_for_clone(conn, false);
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub.update_guest(conn, |guest| {
                    guest.seen_credits = true;
                    guest.won_game = true;
                });
                // ClientboundGameEventPacket.WIN_GAME.
                host.hub.send_to(
                    conn,
                    &C::GameEvent(play::GameEvent {
                        event: 4,
                        param: 0.0,
                    }),
                );
                return;
            }
            if self.guest_portal_cooldown(conn) == 0 {
                if self.common.dim == Dim::End {
                    // Entity.handlePortal sets the cooldown before resolving its destination.
                    self.set_guest_portal_cooldown(conn);
                    self.queue_spawn_trip(conn, SpawnAction::EndExit);
                } else {
                    if !self.server_player_travel_allowed(conn, Dim::End, TravelCause::EndPortal) {
                        return;
                    }
                    // Entity.handlePortal: setPortalCooldown before the teleport.
                    self.set_guest_portal_cooldown(conn);
                    self.start_server_trip(conn, Dim::End, None);
                }
            }
            // Entity.setAsInsidePortal while cooling down restarts the cooldown.
            self.set_guest_portal_cooldown(conn);
            return;
        }
        // EndGatewayBlock.entityInside: a gateway that is not cooling down
        // takes the player at once (no transition time) and cools down
        // (triggerCooldown), even for a player still on its own cooldown.
        if !riding
            && let Some(g) = gateway
            && self.gateway_available(g)
        {
            let ready = self.guest_portal_cooldown(conn) == 0;
            if ready {
                let Some(player) = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                else {
                    return;
                };
                let (id, name) = (player.uuid, player.name.clone());
                let mut cancel = false;
                self.native_player_teleport_end_gateway(id, &name, g, &mut cancel);
                if cancel {
                    return;
                }
                if !self.server_player_travel_allowed(
                    conn,
                    self.common.dim,
                    TravelCause::EndGateway,
                ) {
                    return;
                }
            }
            if self.gateway_enter(g) {
                // setPortalCooldown: before the trip, or again while cooling down.
                self.set_guest_portal_cooldown(conn);
                if ready {
                    self.start_gateway_trip(conn, g);
                }
                return;
            }
        }
        let rule = if invulnerable {
            sunrise_game_data::gamerules::vanilla::PLAYERS_NETHER_PORTAL_CREATIVE_DELAY
        } else {
            sunrise_game_data::gamerules::vanilla::PLAYERS_NETHER_PORTAL_DEFAULT_DELAY
        };
        let transition = self.common.ui.rules.int_at(rule).max(0);
        let Some(entry) = self
            .guest_portal(conn)
            .and_then(|portal| portal.tick_inside(inside, usable, transition, PLAYER_DELAY))
        else {
            return;
        };
        let plan = self.portal_plan(entry, at, WIDTH as f32, HEIGHT as f32, true);
        if !self.may_enter(plan.dim) {
            return;
        }
        if !self.server_player_travel_allowed(conn, plan.dim, TravelCause::NetherPortal) {
            // The processor already reached its threshold. Forget this attempt
            // so a canceled trip must complete a fresh portal wait before retrying.
            if let Some(portal) = self.guest_portal(conn) {
                portal.process = None;
            }
            return;
        }
        self.start_server_trip(conn, plan.dim, Some(plan));
    }

    /// `Entity.handlePortal` of a vehicle server players ride, once through
    /// a portal (a vehicle's transition time is 0): the riders travel with
    /// it and ride it again on arrival (`Entity.teleportCrossDimension`), so
    /// a rider never waits for its own portal delay.
    pub fn carry_riders_through_portals(&mut self) {
        for (vehicle, entry) in self.take_carrying(self.common.dim) {
            let Some(plan) = self.vehicle_plan(vehicle, entry) else {
                continue;
            };
            if !self.may_enter(plan.dim) {
                continue;
            }
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let riders: Vec<u32> = host
                .hub
                .guests
                .iter()
                .filter(|g| {
                    g.vehicle
                        .is_some_and(|mount| self.vehicle_tree_contains_mount(vehicle, mount))
                        && !self.has_pending_travel(g.conn)
                })
                .map(|g| g.conn)
                .collect();
            // Entity.canTeleport: out of the End, every rider has seen the
            // credits.
            if plan.end
                && self.common.dim == Dim::End
                && riders
                    .iter()
                    .any(|c| !host.hub.guest(*c).is_some_and(|g| g.seen_credits))
            {
                continue;
            }
            // Refused passengers are detached and stay in the source level.
            let cause = TravelCause::of_plan(&plan);
            if !self.entity_may_travel(vehicle, plan.dim, cause) {
                continue;
            }
            // An End portal's exit is found as a player's (its own respawn
            // point out of the End).
            let (target, plan) = (plan.dim, (!plan.end).then_some(plan));
            for conn in riders {
                if !self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guest(conn))
                    .is_some_and(|g| {
                        g.vehicle
                            .is_some_and(|mount| self.vehicle_tree_contains_mount(vehicle, mount))
                    })
                {
                    continue;
                }
                self.start_server_trip(conn, target, plan);
            }
        }
    }

    /// A Nether portal block a player's box is in.
    fn nether_portal_inside(&self, at: Vec3) -> Option<BlockPos> {
        let w = WIDTH / 2.0 - 1.0e-5;
        let lo = [at.x - w, at.y + 1.0e-5, at.z - w];
        let hi = [at.x + w, at.y + HEIGHT - 1.0e-5, at.z + w];
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    if &*self.block_name([x, y, z]) == "minecraft:nether_portal" {
                        return Some(BlockPos::new(x, y, z));
                    }
                }
            }
        }
        None
    }

    /// The player moves to `target`'s level at the approximate exit while its chunks load.
    pub fn start_server_trip(&mut self, conn: u32, target: Dim, plan: Option<Plan>) {
        // A player changing level carries no mob a mod seated on it: it gets off here.
        if target != self.common.dim
            && let Some(carrier) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .map(|g| g.entity_id as u32)
        {
            self.eject_player_riders(Some(carrier));
        }
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if target != self.common.dim {
            drop_hook(host, conn);
        }
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let from = Vec3::new(g.position[0], g.position[1], g.position[2]);
        let yaw = g.yaw;
        // The vehicle the player rides travels with it: the player is its
        // passenger, and the vehicle is recreated in the other dimension,
        // where they ride it again (`Entity.teleportCrossDimension`). It
        // keeps its entity id, so the ride is never broken.
        let direct_vehicle = g.vehicle;
        let vehicle = direct_vehicle.filter(|id| {
            self.common.mob_object(*id).is_some_and(|mob| !mob.removed)
                || self.common.boats.iter().any(|b| b.id == *id)
                || self.common.carts.iter().any(|c| c.id == *id)
        });
        let source = self.common.dim;
        let end = plan.is_none();
        // Coordinates scale by the levels' coordinate scales (8 between the Overworld
        // and the Nether); an End portal has fixed destinations.
        let scale = if end {
            1.0
        } else {
            sunrise_world::dimension_type::DimensionFacts::teleportation_scale(
                self.common.dimension_type_of(source),
                self.common.dimension_type_of(target),
            )
        };
        self.server.end_portal_travel = end;
        // Out of the End when the trip starts there, wherever it leads (the world spawn
        // may be in the End too); the credits respawn is always a way out.
        let entering = end && source != Dim::End;
        // Leaving the End: the player's own respawn point.
        let own = if end {
            self.as_guest(conn, |g| g.end_destination(entering, target))
                .flatten()
        } else {
            None
        };
        let scaled = own
            .or_else(|| self.end_destination(entering, target))
            .unwrap_or(Vec3::new(from.x * scale, from.y, from.z * scale));
        self.server.end_portal_travel = false;
        let (retired_window, mut menu_hold) = if source != target {
            let (window, hold) = self.guest_close_menus_for_travel(conn);
            (window, Some(hold))
        } else {
            (None, None)
        };
        if !self.enter_travel_destination(conn, target, retired_window, &mut menu_hold) {
            return;
        }
        let Some(i) = self.guest_index(conn) else {
            if let Some(hold) = menu_hold {
                self.resume_menu_opens(hold);
            }
            if let Err(e) = self.enter_level(source) {
                log::error!("could not reopen {}: {e}", source.id());
            }
            return;
        };
        let destination = match plan {
            Some(plan) => self.arrival_centre(&plan),
            None => scaled,
        };
        let at = [destination.x, destination.y, destination.z];
        let ctx = server_ctx!(self);
        // Through a portal: the client waits on its loading screen until
        // the exit is found (`arrive_server_trips`). The cooldown is the one
        // Entity.handlePortal set before the teleport.
        let Some(host) = self.server.lan.as_mut() else {
            if let Some(hold) = menu_hold {
                self.resume_menu_opens(hold);
            }
            return;
        };
        let change = host
            .hub
            .begin_level_change(i, at, yaw, None, PLAYER_DELAY, &ctx);
        self.send_player_command_tree(conn);
        let ctx = server_ctx!(self);
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.finish_level_change(i, change, true, &ctx);
        }
        self.bind_player_attachments(conn, false);
        if let Some(hold) = menu_hold {
            self.resume_menu_opens(hold);
        }
        self.as_guest(conn, |g| {
            g.trigger_dimension(source, target, [from.x, from.y, from.z]);
            // ServerPlayer.teleport: stopUsingItem after the dimension change triggers.
            if source != target {
                g.stop_active_item_use();
            }
        });
        self.server.server_trips.push(Trip {
            conn,
            dim: target,
            source,
            target: destination,
            end,
            entering,
            plan,
            vehicle,
            gateway: None,
            respawn: None,
            spawn_search: None,
            world_spawn: false,
            missing: None,
            finder: None,
            placed: false,
            hold: false,
            left: Some(source),
        });
        if let Err(e) = self.enter_level(source) {
            log::error!("could not reopen {}: {e}", source.id());
        }
    }

    /// Menu removal is silent on a successful trip: the respawn closes the
    /// old screen. On failure, close it explicitly unless a listener opened
    /// a replacement during removal.
    fn enter_travel_destination(
        &mut self,
        conn: u32,
        dim: Dim,
        retired_window: Option<i32>,
        menu_hold: &mut Option<super::lan_menus::MenuOpenHold>,
    ) -> bool {
        let Err(e) = self.enter_level(dim) else {
            return true;
        };
        log::error!("could not open {}: {e}", dim.id());
        if let Some(hold) = menu_hold.take() {
            self.resume_menu_opens(hold);
        }
        if let Some(container) = retired_window
            && let Some(host) = self.server.lan.as_mut()
            && super::native_menus::open_menu_of(host, conn).is_none()
        {
            host.hub
                .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
        }
        false
    }

    /// Both PlayerList.respawn and an ordinary End exit use the same spawn search. Its
    /// chunks must load before either the replacement or the teleport moves the player.
    pub(in crate::game::server_side) fn queue_spawn_trip(
        &mut self,
        conn: u32,
        action: SpawnAction,
    ) {
        if self.server.server_trips.iter().any(|t| {
            t.is(conn)
                && t.spawn_search
                    .as_ref()
                    .is_some_and(|search| matches!(search.action, SpawnAction::Replace { .. }))
        }) {
            return;
        }
        let Some(guest) = self.server.lan.as_ref().and_then(|h| h.hub.guest(conn)) else {
            return;
        };
        let source = Dim::from_id(&guest.dimension);
        let config = guest
            .respawn_elsewhere
            .as_ref()
            .and_then(RespawnConfig::from_saved);
        let (dim, target, world_spawn, legacy_personal_point) = if let Some(c) = config {
            let [x, y, z] = c.pos.map(f64::from);
            (c.dim, Vec3::new(x + 0.5, y, z + 0.5), false, false)
        } else if let Some([x, y, z]) = guest.spawn {
            (
                Dim::Overworld,
                Vec3::new(x, y, z),
                false,
                matches!(action, SpawnAction::EndExit),
            )
        } else {
            let [x, y, z] = self.world_spawn_position();
            (
                self.world_spawn_level(),
                Vec3::new(x, y, z),
                self.common.level.world_spawn.is_some(),
                false,
            )
        };
        let return_to = self.common.dim;
        if let Err(e) = self.enter_level(dim) {
            log::error!("could not open {} for player spawn search: {e}", dim.id());
            return;
        }
        self.server.server_trips.retain(|t| !t.is(conn));
        let trip = Trip {
            conn,
            dim,
            source,
            target,
            end: false,
            entering: false,
            plan: None,
            vehicle: None,
            gateway: None,
            respawn: config,
            spawn_search: Some(SpawnTrip {
                action,
                waiting_for: None,
                legacy_personal_point,
            }),
            world_spawn,
            missing: None,
            finder: None,
            placed: false,
            hold: false,
            left: None,
        };
        if (config.is_none() && !world_spawn && !legacy_personal_point)
            || self.trip_destination_ready(&trip)
        {
            if let Some(pending) = self.resolve_spawn_trip(trip, action) {
                self.server.server_trips.push(pending);
            }
        } else {
            self.server.server_trips.push(trip);
        }
        if let Err(e) = self.enter_level(return_to) {
            log::error!("could not reopen {}: {e}", return_to.id());
        }
    }

    fn trip_destination_ready(&self, trip: &Trip) -> bool {
        let (cx, cz) = Self::chunk_of(trip.target);
        (-2..=2).all(|dz| (-2..=2).all(|dx| self.server.streaming.is_lit(cx + dx, cz + dz)))
            && trip
                .spawn_search
                .as_ref()
                .and_then(|search| search.waiting_for)
                .is_none_or(|(x, z)| self.server.streaming.is_lit(x, z))
    }

    fn resolve_spawn_trip(&mut self, mut trip: Trip, action: SpawnAction) -> Option<Trip> {
        let before = self.common.dim;
        let consume = matches!(action, SpawnAction::Replace { was_death: true });
        let legacy_personal_point = trip
            .spawn_search
            .as_ref()
            .is_some_and(|search| search.legacy_personal_point);
        let result = (|| -> sunrise_game_data::Result<Option<Trip>> {
            let own = if let Some(config) = trip.respawn {
                if let Some(found) = self.find_respawn_and_use(config, consume)? {
                    Some(found)
                } else {
                    // TeleportTransition.missingRespawnBlock: search the world's spawn before
                    // placing the player, without an intermediate move into the config's level.
                    let [x, y, z] = self.world_spawn_position();
                    trip.dim = self.world_spawn_level();
                    trip.target = Vec3::new(x, y, z);
                    trip.respawn = None;
                    trip.missing = Some(config);
                    trip.world_spawn = self.common.level.world_spawn.is_some();
                    self.enter_level(trip.dim)?;
                    if trip.world_spawn && !self.trip_destination_ready(&trip) {
                        return Ok(Some(trip));
                    }
                    None
                }
            } else {
                None
            };
            let (at, yaw, pitch) = if let Some(found) = own {
                found
            } else {
                let landing = if legacy_personal_point {
                    self.end_exit_landing(trip.target, &mut trip.finder)
                } else if trip.world_spawn {
                    let suggestion =
                        [trip.target.x, trip.target.y, trip.target.z].map(|v| v.floor() as i32);
                    self.search_spawn(suggestion, &mut trip.finder)
                } else {
                    Ok(trip.target)
                };
                let at = match landing {
                    Ok(at) => at,
                    Err(chunk) => {
                        if let Some(search) = &mut trip.spawn_search {
                            search.waiting_for = Some(chunk);
                        }
                        return Ok(Some(trip));
                    }
                };
                let [yaw, pitch] = self.common.level.world_spawn_angle;
                (at, yaw, pitch)
            };
            let found = Respawn {
                dim: trip.dim,
                at,
                yaw,
                pitch,
            };
            self.enter_level(trip.source)?;
            match action {
                SpawnAction::Replace { was_death } => {
                    self.remove_old_player_for_clone(trip.conn, was_death);
                    let decision =
                        self.respawn_position(trip.conn, found, trip.missing, !was_death);
                    let (to, copy) =
                        decision.map_or((found, trip.missing.is_none()), |d| (d.to, d.copy));
                    self.enter_level(to.dim)?;
                    self.place_cloned_player(
                        trip.conn,
                        to,
                        was_death,
                        copy,
                        trip.missing.is_some(),
                    );
                }
                SpawnAction::EndExit => {
                    // EndPortalBlock returns DO_NOTHING; ServerPlayer.teleport sees only this
                    // resolved transition, including when the saved block was missing.
                    if self.server.lan.as_ref().is_some_and(|host| {
                        host.hub
                            .guest(trip.conn)
                            .is_some_and(|g| host.hub.here(g) && !g.dead)
                    }) && self.server_player_travel_allowed(
                        trip.conn,
                        found.dim,
                        TravelCause::EndPortal,
                    ) {
                        self.apply_level_teleport(
                            LevelTeleport {
                                conn: trip.conn,
                                dim: found.dim,
                                at: [found.at.x, found.at.y, found.at.z],
                                rotation: (!legacy_personal_point)
                                    .then_some([found.yaw, found.pitch]),
                                vehicle: None,
                            },
                            LevelTeleportKind::EndExit {
                                missing: trip.missing.is_some(),
                            },
                        );
                    }
                }
            }
            Ok(None)
        })();
        if let Err(e) = self.enter_level(before) {
            log::error!("could not reopen {}: {e}", before.id());
        }
        match result {
            Ok(pending) => pending,
            Err(e) => {
                log::error!("could not resolve player spawn destination: {e}");
                None
            }
        }
    }

    /// `TheEndGatewayBlockEntity.getPortalPosition` and the teleport within
    /// the End (keeping the player's motion and look): straight to an exact
    /// exit, else once the destination has loaded around the player, who
    /// waits there (its exit is found among those blocks, or built).
    pub fn start_gateway_trip(&mut self, conn: u32, gateway: [i32; 3]) {
        let Some(i) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
        else {
            return;
        };
        let (job, destination) = self.gateway_job(gateway);
        let at = [destination.x, destination.y, destination.z];
        // Relative.union(DELTA, ROTATION).
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.teleport_keeping_motion(i, at);
        }
        if let GatewayJob::Known { exact: true, .. } = job {
            return;
        }
        self.server.server_trips.push(Trip {
            conn,
            dim: self.common.dim,
            source: self.common.dim,
            target: destination,
            end: false,
            entering: false,
            plan: None,
            vehicle: None,
            gateway: Some(job),
            respawn: None,
            spawn_search: None,
            world_spawn: false,
            missing: None,
            finder: None,
            placed: false,
            hold: false,
            left: None,
        });
    }

    /// The trips into the level in place whose destination has loaded
    /// around their target: the exit (the Nether portal found or built, the
    /// End platform, the Overworld's spawn column) and the teleport there.
    fn arrive_server_trips(&mut self) {
        self.follow_command_arrivals();
        let mut k = 0;
        while k < self.server.server_trips.len() {
            let trip = &self.server.server_trips[k];
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == trip.conn) else {
                // They left on the way.
                self.server.server_trips.remove(k);
                continue;
            };
            if trip.holds_movement() {
                self.server.lan.as_mut().unwrap().hub.guests.update(i, |g| {
                    g.landing = true;
                    g.tp_held = true;
                });
            }
            if trip.dim != self.common.dim
                || !self.trip_destination_ready(trip)
                || trip
                    .gateway
                    .and_then(GatewayJob::source_chunk)
                    .is_some_and(|c| !self.common.world.chunks.contains_key(&c))
            {
                k += 1;
                continue;
            }
            let mut trip = self.server.server_trips.remove(k);
            if let Some(action) = trip.spawn_search.as_ref().map(|search| search.action) {
                if let Some(pending) = self.resolve_spawn_trip(trip, action) {
                    self.server.server_trips.insert(k, pending);
                    k += 1;
                }
                continue;
            }
            if trip.placed {
                self.tell_mods_changed_dimension(trip.conn, trip.left, trip.dim);
                continue;
            }
            // A portal's trip (`start_server_trip`): its client waits to land.
            let portal = trip.end || trip.plan.is_some();
            self.resolve_arrivals();
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let yaw = host.hub.guests[i].yaw;
            let (position, yaw) = if let Some(job) = trip.gateway {
                match self.resolve_gateway(job, trip.target) {
                    Ok(at) => (at, yaw),
                    Err(e) => {
                        log::error!("End gateway exit: {e}");
                        (trip.target, yaw)
                    }
                }
            } else if trip.end {
                let entering = trip.entering;
                let guest_arrival = self.as_guest(trip.conn, |g| {
                    g.end_arrival(entering, trip.target, &mut trip.finder)
                });
                let arrival = match guest_arrival {
                    Some(arrival) => arrival,
                    None => self.end_arrival(entering, trip.target, &mut trip.finder),
                };
                match arrival {
                    Ok(Some((at, [turned, tilted]))) => {
                        // EndPortalBlock.getPortalDestination: PLACE_PORTAL_TICKET.
                        self.place_portal_ticket(Self::chunk_of(at));
                        if let Some(pitch) = tilted
                            && let Some(host) = self.server.lan.as_mut()
                        {
                            host.hub.guests.update(i, |guest| guest.pitch = pitch);
                        }
                        (at, turned.unwrap_or(yaw))
                    }
                    Ok(None) => {
                        self.server.server_trips.insert(k, trip);
                        k += 1;
                        continue;
                    }
                    Err(e) => {
                        log::error!("End arrival: {e}");
                        (trip.target, yaw)
                    }
                }
            } else {
                let exit = trip
                    .plan
                    .as_ref()
                    .and_then(|plan| self.nether_exit_portal(plan));
                match (trip.plan, exit) {
                    (Some(plan), Some(exit)) => {
                        let (at, rotation) = self.arrival(&plan, exit);
                        (at, yaw + rotation)
                    }
                    _ => (trip.target, yaw),
                }
            };
            // The vehicle the player rode comes with it, and waits for it
            // where it arrives (`Entity.teleportCrossDimension` carries the
            // passengers).
            if let Some(vehicle) = trip.vehicle {
                self.arrive_vehicle(trip.source, vehicle, position, yaw);
            }
            // Player.getDimensionChangingDelay.
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            if !trip.hold {
                host.play.entry(trip.conn).or_default().portal.cooldown = 10;
            }
            host.hub.guests.update(i, |g| {
                g.yaw = yaw;
                // A new body at the exit: no fall distance or motion carried over,
                // but the player's attributes (ServerPlayer.teleport keeps the entity).
                if !trip.hold {
                    g.new_body(true);
                }
            });
            let at = [position.x, position.y, position.z];
            if portal {
                host.hub.land(i, at);
            } else {
                host.hub.teleport(i, Some(at));
            }
            self.tell_mods_changed_dimension(trip.conn, trip.left, trip.dim);
        }
        self.release_held_arrivals();
    }

    /// The vehicle the player rode arrives with it, at the point the player
    /// arrives: it keeps its entity id, so the ride is never broken
    /// (`Entity.teleportCrossDimension` recreates the vehicle in the other
    /// dimension, and its passengers ride it again).
    pub fn arrive_vehicle(&mut self, source: Dim, id: u32, at: Vec3, yaw: f32) {
        let Some(id) = self.parked_vehicle_root(source, id) else {
            return;
        };
        let motion = self
            .server
            .portals
            .carried_motion
            .remove(&id)
            .unwrap_or(Vec3::ZERO);
        let carried = self.server.portals.carried_cooldown.remove(&id);
        let Some(parked) = self.server.levels.0.parked_mut(source) else {
            return;
        };
        let before = parked
            .boats
            .iter()
            .find(|body| body.id == id)
            .map(|body| (body.position, [body.yaw, 0.0]))
            .or_else(|| {
                parked
                    .carts
                    .iter()
                    .find(|body| body.id == id)
                    .map(|body| (body.position, [body.yaw, 0.0]))
            })
            .or_else(|| {
                parked
                    .mobs
                    .iter()
                    .find(|body| body.id == id)
                    .map(|body| (body.position, [body.yaw, body.head_pitch]))
            });
        let Some((from, rotation)) = before else {
            return;
        };
        // Keep the root out of both levels until its passengers have been
        // admitted: teleportCrossDimension copies passengers before the root.
        let vehicle = if let Some(body) = take_vehicle(&mut parked.boats, id, at, yaw, motion) {
            OffLevel::Boat(body)
        } else if let Some(body) = take_vehicle(&mut parked.carts, id, at, yaw, motion) {
            OffLevel::Cart(body)
        } else if let Some(body) = take_vehicle(&mut parked.mobs, id, at, yaw, motion) {
            OffLevel::Mob(body)
        } else {
            return;
        };
        let left = parked
            .portal_states
            .remove(&id)
            .map_or(0, |state| state.cooldown);
        // All object-owned metadata moves, including rejected intermediate bodies;
        // ordinary destination queries still expose only admitted objects.
        for mut mob in self.move_entity_memory(source, id) {
            let destination =
                super::vehicle_teleport::passenger_destination(from, at, mob.position);
            sunrise_sim::mobs::living::teleports::snap_to(&mut mob, destination);
            let [turned_yaw, pitch] = super::vehicle_teleport::passenger_rotation(
                rotation,
                [yaw, rotation[1]],
                [mob.yaw, mob.head_pitch],
            );
            mob.yaw = turned_yaw;
            mob.previous_yaw = turned_yaw;
            mob.body_yaw = turned_yaw;
            mob.head_yaw = turned_yaw;
            mob.head_pitch = pitch;
            mob.previous_head_pitch = pitch;
            self.admit_transferred_object(OffLevel::Mob(mob));
        }
        self.admit_transferred_object(vehicle);
        // The source may have ticked while the player's destination was loading.
        self.server.portals.entities.entry(id).or_default().cooldown = carried.unwrap_or(left);
    }

    /// Tells the mods of the teleports a command made inside a level, with
    /// each player's state swapped in: where the move has reached the hub.
    pub(in crate::game::server_side) fn flush_teleport_notes(&mut self) {
        for note in std::mem::take(&mut self.server.teleport_notes) {
            let TeleportNote {
                conn,
                uuid,
                name,
                from,
                to,
                dimension,
            } = note;
            // In place when it is their turn already (as_guest finds them swapped in).
            if self.server.lan_acting == Some(conn) {
                self.native_player_teleported(uuid, &name, from, to, &dimension);
            } else {
                self.as_guest(conn, |g| {
                    g.native_player_teleported(uuid, &name, from, to, &dimension);
                });
            }
        }
    }

    /// The players the command just run sends to another level
    /// (`ServerPlayer.teleport` from TeleportCommand): a Respawn packet
    /// keeping all data and the teleport, with no portal sound. The server
    /// leaves the body alone until the level has loaded around it.
    pub fn run_level_teleports(&mut self) {
        if self.server.lan.is_none() {
            return;
        }
        for t in std::mem::take(&mut self.server.level_teleports) {
            self.apply_direct_level_teleport(t);
        }
    }

    pub(in crate::game::server_side) fn apply_direct_level_teleport(
        &mut self,
        teleport: LevelTeleport,
    ) {
        self.apply_level_teleport(teleport, LevelTeleportKind::Direct);
    }

    fn apply_level_teleport(&mut self, t: LevelTeleport, kind: LevelTeleportKind) {
        let back = self.common.dim;
        let Some(player) = self.command_guest(t.conn) else {
            return;
        };
        let source = Dim::from_id(&player.dimension);
        let id = player.entity_id as u32;
        if let Err(e) = self.enter_level(source) {
            log::error!("could not open {}: {e}", source.id());
            return;
        }
        if let LevelTeleportKind::EndExit { missing } = kind
            && let Some(host) = self.server.lan.as_mut()
        {
            if missing {
                host.hub.missing_respawn_block(t.conn);
            }
            host.play.entry(t.conn).or_default().portal.cooldown = PLAYER_DELAY;
            if source == t.dim {
                // ServerPlayer.teleport within a level applies the absolute destination
                // and sends only the position packet (no Respawn or command notification).
                let [x, y, z] = t.at;
                let _ = self.as_guest(t.conn, |g| {
                    g.place_player_teleport_in_level(Vec3::new(x, y, z), t.rotation, [false; 3])
                });
                if let Err(e) = self.enter_level(back) {
                    log::error!("could not reopen {}: {e}", back.id());
                }
                return;
            }
            self.eject_player_riders(Some(id));
        }
        let (retired_window, mut menu_hold) = if t.dim != source {
            let (window, hold) = self.guest_close_menus_for_travel(t.conn);
            if let Some(host) = self.server.lan.as_mut() {
                drop_hook(host, t.conn);
            }
            (window, Some(hold))
        } else {
            (None, None)
        };
        let index = if self.enter_travel_destination(t.conn, t.dim, retired_window, &mut menu_hold)
        {
            self.guest_index(t.conn)
        } else {
            None
        };
        let Some(i) = index else {
            if let Some(hold) = menu_hold {
                self.resume_menu_opens(hold);
            }
            if let Err(e) = self.enter_level(back) {
                log::error!("could not reopen {}: {e}", back.id());
            }
            return;
        };
        let ctx = server_ctx!(self);
        let Some(host) = self.server.lan.as_mut() else {
            if let Some(hold) = menu_hold {
                self.resume_menu_opens(hold);
            }
            return;
        };
        let guest = &host.hub.guests[i];
        let [yaw, pitch] = t.rotation.unwrap_or([guest.yaw, guest.pitch]);
        let origin = guest.position;
        let cooldown = host.play.get(&t.conn).map_or(0, |p| p.portal.cooldown);
        let change = host
            .hub
            .begin_level_change(i, t.at, yaw, Some(pitch), cooldown, &ctx);
        self.send_player_command_tree(t.conn);
        let ctx = server_ctx!(self);
        if let Some(host) = self.server.lan.as_mut() {
            let passenger_body = host.hub.guests.update(i, |g| {
                // A new body there: no fall distance or motion carried over, but
                // the player's attributes (ServerPlayer.teleport keeps the entity).
                if t.vehicle.is_some() {
                    g.motion.take()
                } else {
                    g.new_body(true);
                    None
                }
            });
            host.hub.finish_level_change(i, change, false, &ctx);
            host.hub.guests.update(i, |g| {
                g.vehicle = t.vehicle;
                if let Some(mut body) = passenger_body {
                    body.position = Vec3::new(t.at[0], t.at[1], t.at[2]);
                    g.motion = Some(body);
                }
            });
        }
        self.bind_player_attachments(t.conn, false);
        if let Some(hold) = menu_hold {
            self.resume_menu_opens(hold);
        }
        match kind {
            LevelTeleportKind::EndExit { .. } => {
                self.as_guest(t.conn, |g| {
                    g.trigger_dimension(source, t.dim, origin);
                    g.stop_active_item_use();
                });
                self.tell_mods_changed_dimension(t.conn, Some(source), t.dim);
            }
            LevelTeleportKind::Direct => {
                if let Some(player) = self.command_guest(t.conn) {
                    let (uuid, name) = (player.uuid, player.name.clone());
                    self.as_guest(t.conn, |g| {
                        g.trigger_dimension(source, t.dim, origin);
                        g.native_player_teleported(uuid, &name, origin, t.at, t.dim.id());
                    });
                }
                // `ServerPlayer.teleport` has already placed the player in `t.dim`;
                // fire `player_changed_dimension` now (just like `EndExit` above) so
                // consecutive direct cross-dimension teleports in one tick never lose
                // the first transition when `self.server.server_trips.retain` replaces the trip.
                self.tell_mods_changed_dimension(t.conn, Some(source), t.dim);
                self.server.server_trips.retain(|trip| !trip.is(t.conn));
                self.server.server_trips.push(Trip {
                    conn: t.conn,
                    dim: t.dim,
                    source,
                    target: Vec3::new(t.at[0], t.at[1], t.at[2]),
                    end: false,
                    entering: false,
                    plan: None,
                    // A command teleports the player alone: it is not a
                    // passenger any more (`ServerPlayer.teleport`).
                    vehicle: None,
                    gateway: None,
                    respawn: None,
                    spawn_search: None,
                    world_spawn: false,
                    missing: None,
                    finder: None,
                    placed: true,
                    // The body does not tick until arrival. Letting it leave the destination before
                    // its chunks load can strand this trip and freeze its damage cooldown indefinitely.
                    hold: true,
                    left: None,
                });
            }
        }
        if let Err(e) = self.enter_level(back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
    }

    /// `native_player_changed_dimension` for a guest who has ended a move
    /// from the level `left` to the one in place, `to`, with their state
    /// swapped in so that the helpers reach them. Nothing is heard when they
    /// did not leave a level, or ended where they started.
    pub(in crate::game::server_side) fn tell_mods_changed_dimension(
        &mut self,
        conn: u32,
        left: Option<Dim>,
        to: Dim,
    ) {
        let Some(from) = left.filter(|from| *from != to) else {
            return;
        };
        let Some((uuid, name)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        self.as_guest(conn, |g| {
            g.native_player_changed_dimension(uuid, &name, from.id(), to.id());
        });
    }

    /// The hub's view of the server for packets sent outside its own tick.
    #[cfg(any(test, feature = "test-support"))]
    pub fn server_ctx(&mut self) -> TickCtx<'_> {
        let spawn = self.world_spawn_position();
        let clock_rate = self.network_clock_rate();
        TickCtx {
            sea_level: self
                .common
                .end
                .locator()
                .map_or(63, |terrain| terrain.sea_level()),
            world: &mut self.common.world,
            entities: &[],
            next_entity: &mut self.server.next_entity,
            spawn,
            spawn_angle: self.common.level.world_spawn_angle,
            spawn_dimension: sunrise_gameplay_common::game::Dim::of_world_spawn(&self.common.level)
                .id(),
            difficulty: i32::from(self.common.ui.difficulty),
            difficulty_locked: self.common.level.difficulty_locked,
            clock_rate,
            tick_rate: self.common.clock.rate(),
            tick_frozen: self.common.clock.frozen(),
            tick_steps: self.common.clock.step_state(),
            collision: Some(self.common.collision),
            border: &self.common.border,
            streaming: Some(&self.server.streaming),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A minecart a player rides through a portal rolls on at the motion it
    /// crossed with, clamped to 0.4 along x and z (`PositionMoveRotation.of`
    /// takes `OldMinecartBehavior.getKnownMovement`), not at rest. Vanilla
    /// 26.3 sent the client a cart arriving with motion (0.4, 0, 0).
    #[test]
    fn a_minecart_a_player_rides_keeps_its_clamped_motion_through_a_portal() {
        let mut carts: sunrise_sim::join_seq::JoinList<_> = [minecarts::Minecart::new(
            3,
            minecarts::CartKind::Rideable,
            Vec3::new(9.9, 100.0, 4.5),
        )]
        .into_iter()
        .collect();
        let at = Vec3::new(1.5, 100.0, 0.5);
        let cart = take_vehicle(&mut carts, 3, at, 0.0, Vec3::new(1.6, -0.1, -0.7))
            .expect("the cart travels");
        assert_eq!(cart.position, at);
        assert_eq!(cart.velocity, Vec3::new(0.4, -0.1, -0.4));
    }

    /// The vehicle a player rides travels with it through a portal: it
    /// leaves the level the player came from, keeps its entity id (so the
    /// ride is never broken) and stands where its rider arrives
    /// (`Entity.teleportCrossDimension` carries the passengers, and they
    /// ride the vehicle that is recreated in the other dimension).
    #[test]
    fn the_vehicle_a_player_rides_travels_with_it() {
        let mut boats: sunrise_sim::join_seq::JoinList<_> =
            [sunrise_gameplay_common::game::boats_common::Boat::new(
                7,
                Arc::from("oak"),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            )]
            .into_iter()
            .collect();
        let mut carts = sunrise_sim::join_seq::JoinList::<minecarts::Minecart>::default();
        let mut mobs = sunrise_sim::join_seq::JoinList::<Mob>::default();
        let at = Vec3::new(1.5, 65.0, -2.5);
        let boat = take_vehicle(&mut boats, 7, at, 90.0, Vec3::ZERO)
            .expect("the vehicle the player rides travels with it");
        assert!(
            boats.is_empty(),
            "the vehicle left the level the player came from"
        );
        assert_eq!(boat.id, 7, "the vehicle keeps its entity id");
        assert_eq!(boat.position, at, "it stands where its rider arrives");
        assert_eq!(boat.yaw, 90.0, "it looks where its rider looks");
        assert_eq!(boat.velocity, Vec3::ZERO, "it comes with no motion");
        // A vehicle that is not in the level does not travel.
        assert!(take_vehicle(&mut carts, 9, at, 0.0, Vec3::ZERO).is_none());
        assert!(carts.is_empty());
        assert!(take_vehicle(&mut mobs, 9, at, 0.0, Vec3::ZERO).is_none());
        assert!(mobs.is_empty());
    }
}

#[cfg(test)]
mod transfer_lifetime_tests {
    use sunrise_gameplay_common::game::server_side::server_travel::transfer_lifetime_tests::VALUE;

    use super::*;
    use std::sync::{Arc, Mutex};
    use sunrise_mod_runtime::attachments::{
        Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
    };

    fn removal<T: Rideable + sunrise_sim::join_seq::Joined + HasAttachmentLifetime>(mut owner: T) {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Entity(owner.id()), 17).unwrap();
        owner.attachment_lifetime().bind(&mut map, &index, 42u32);
        let mut list: sunrise_sim::join_seq::JoinList<T> = [owner].into_iter().collect();
        let id = list[0].id();
        let key = ("test".into(), "transfer".into());
        let retained = index.lock().unwrap().snapshot(&key);
        let moved = take_vehicle(&mut list, id, Vec3::ZERO, 0.0, Vec3::ZERO).unwrap();
        assert!(list.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut map, Holder::Entity(id), 18).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(retained[0].entry.read(&VALUE), Ok(&17));
        drop(moved);
    }

    #[test]
    fn transfer_removal_retires_before_returning_the_vehicle() {
        removal(minecarts::Minecart::new(
            71,
            minecarts::CartKind::Rideable,
            Vec3::ZERO,
        ));
        removal(sunrise_gameplay_common::game::boats_common::Boat::new(
            72,
            Arc::from("oak"),
            Vec3::ZERO,
            0.0,
        ));
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod credits_respawn_tests {
    use sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::ANCHOR;

    use sunrise_level::chunk::Chunk;
    use sunrise_world::registry::BlockStateId;

    pub fn light_spawn_chunks(game: &mut crate::game::ServerGame<'_>) {
        for x in -2..=2 {
            for z in -2..=2 {
                game.server
                    .streaming
                    .insert_remote(&mut game.common.world, Chunk::empty(x, z));
            }
        }
    }

    pub fn charge_anchor(game: &mut crate::game::ServerGame<'_>) -> BlockStateId {
        let registry = game.pack.assets.registry;
        let id = registry.default_state("minecraft:respawn_anchor").unwrap();
        let mut value = registry.state(id).unwrap().clone();
        value.properties.insert("charges".into(), "1".into());
        let charged = registry.resolve(&value).unwrap();
        let [x, y, z] = ANCHOR;
        game.world.set(x, y, z, charged).unwrap();
        charged
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
