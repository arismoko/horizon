//! The `entity_position` area: where an entity is and which way it faces, and moving it between places. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_position` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityPositionBackend {
    /// Side: both.
    /// The entity's position (`Entity.position`): its feet, the middle of
    /// its box for a painting or an item frame. A client display uses its current tick
    /// position after movement interpolation, not the packet destination or draw-frame
    /// pose. Returns `None` for an unresolved handle. An experience orb visited in
    /// a parked level can be read without putting its level in place.
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: both.
    /// The block the entity's position is in (`Entity.blockPosition`), derived
    /// from `entity_position`: on a client, answers for the local player and
    /// tracked entities (including displays). API 373.
    fn entity_block_position(&self, entity: EntityRef) -> Option<[i32; 3]>;
    /// Side: both.
    /// The entity's yaw and pitch in degrees (`Entity.getYRot`, `getXRot`); 0
    /// for the kinds that don't turn. API 373.
    fn entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]>;
    /// Side: both.
    /// Where the entity was when its current tick began (`Entity.xo`, `yo`, `zo`): what
    /// `Entity.setOldPosAndRot` stored in `Entity.commonTick`, which the level's entity pass
    /// (`ServerLevel.tickNonPassenger`, `ClientLevel.tickNonPassenger`, and `tickPassenger` for
    /// a rider) runs before the entity's own tick, and a teleport or a load also sets. Between
    /// two ticks it is the position the last tick started from, so a move this tick made goes
    /// from here to [`EntityRef::position`]; [`EntityRef::position_at`] eases between them as
    /// `Entity.getPosition(float)` does. NeoForge mods read the same vanilla fields.
    ///
    /// A server's player is snapped wherever the server moves it (`absSnapTo` for an accepted
    /// move and after `ServerGamePacketListenerImpl.tickPlayer`), so it reads its position.
    /// On a client, the local player's own (where the camera eases from), and the client's
    /// copy of every entity it ticks; an entity the client tracks without a body of its own
    /// answers from the copy [`EntityRef::position`] reads. None for a stale handle or an
    /// entity whose body the client doesn't keep.
    fn entity_previous_position(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: both.
    /// The yaw and pitch in degrees when the entity's current tick began (`Entity.yRotO`,
    /// `xRotO`), in the order of [`EntityRef::rotation`]: what `Entity.setOldPosAndRot` stored,
    /// as for [`EntityRef::previous_position`]. `Entity.turn` moves them with the view, and a
    /// projectile's `ProjectileUtil.rotateTowardsMovement` and `ProjectileDeflection.REVERSE`
    /// shift them, as vanilla does. A renderer eases from here to the rotation
    /// (`Entity.getViewYRot(float)`, `getViewXRot(float)`). NeoForge mods read the same vanilla
    /// fields. On a client, the local player's and those of every entity the client keeps a
    /// body for, as [`EntityRef::rotation`] answers. None for a stale handle or an entity
    /// whose body the client doesn't keep.
    fn entity_previous_rotation(&self, entity: EntityRef) -> Option<[f32; 2]>;
    /// Side: both.
    /// Whether the entity stands on a block (`Entity.onGround`). API 379.
    /// On a client, the local player's own predicted flag (the one it sends); for another
    /// kept entity, including a tracked player, its client copy's flag. Neither is server
    /// authority. None for a stale handle or an entity whose body the client doesn't keep.
    fn entity_on_ground(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity's last collided move was stopped on a horizontal axis
    /// (`Entity.horizontalCollision`): a general entity row's, written by every `Entity.move`
    /// of any cause and cleared with it when the row has `noPhysics`. None for a stale handle
    /// and for the other kinds, whose flag Sunrise keeps inside their own tick.
    fn entity_horizontal_collision(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity's last collided move was stopped vertically
    /// (`Entity.verticalCollision`): a general entity row's, as for
    /// [`EntityRef::horizontal_collision`]. None for a stale handle and for the other kinds.
    fn entity_vertical_collision(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// `Entity.getOnPos()`: the supporting block when available, otherwise the feet block with
    /// vanilla's `1.0E-5F` subtracted from Y. Simulated bodies with retained support use
    /// `Entity.mainSupportingBlockPos` from their last support check, not a new collision query.
    /// A dragon part, which never stands, has the block under its feet. None for a stale handle.

    fn entity_on_block(&self, entity: EntityRef) -> Option<[i32; 3]>;
    /// Side: server.
    /// The square of the distance from the entity's feet to `pos`
    /// (`Entity.distanceToSqr`). API 379.
    fn entity_distance_to_sqr(&self, entity: EntityRef, pos: [f64; 3]) -> Option<f64>;
    /// Side: server.
    /// The yaw of the entity's head (`Entity.getYHeadRot`): a mob's or a
    /// player's own, zero for the kinds without a head.
    fn entity_head_yaw(&self, entity: EntityRef) -> Option<f32>;
    /// Side: both.
    /// The entity's current body yaw (`LivingEntity.yBodyRot`) in degrees, without wrapping.
    /// On a server this reads the named living entity. On a client it reads the local player.
    /// None for a stale handle or an entity with no available living body.
    fn entity_body_yaw(&self, entity: EntityRef) -> Option<f32>;
    /// Side: server.
    /// The horizontal direction the entity faces (`Entity.getDirection`):
    /// `north`, `south`, `east` or `west`.
    fn entity_direction(&self, entity: EntityRef) -> Option<&'static str>;
}
/// The handles' writes of the `entity_position` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityPositionBackendMut {
    /// Side: server.
    /// Sets the current body yaw in degrees (`LivingEntity.setYBodyRot`). It does not
    /// wrap the angle, teleport the entity, or force a packet. Silverfish and endermites
    /// also turn their view; armor stands change their previous body/view yaw and their
    /// head yaw, leaving their current body yaw alone. False for stale handles or when the
    /// server keeps no living body for this entity. Returns `Err(LevelWriteError::Unavailable)`
    /// when the host cannot write living state.
    fn entity_set_body_yaw(&mut self, entity: EntityRef, yaw: f32)
    -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// `Entity.setOnGround`: set the live body's ground flag and check supporting
    /// blocks under its current footprint without moving it. False clears retained
    /// support; true retains the previous block on the first miss and clears it on
    /// a second miss. Does not change velocity, fall distance, or collision flags.
    /// `Ok(false)` for stale handles and bodies that don't keep ground state.
    /// If the support scan fails, returns `Err(LevelWriteError::Unavailable)`;
    /// the flag is still written, and the previous supporting block is retained.
    fn entity_set_on_ground(
        &mut self,
        entity: EntityRef,
        on_ground: bool,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Moves the entity to `pos` in its level (`Entity.teleportTo`), as
    /// `/tp` does: a player is placed there and told, a rider gets off, and a
    /// vehicle (including a general entity) takes its riders with it. With
    /// `rotation` (yaw, pitch in degrees) it also turns. False for a position
    /// outside the world's bounds, for wall-mounted entities or dragon parts, or when the
    /// requested rotation cannot be stored.
    fn entity_teleport_to(
        &mut self,
        entity: EntityRef,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool;
    /// Side: server.
    /// Moves the entity by `delta` blocks as `Entity.move(cause, delta)` does: the block,
    /// entity and world border collision scan stops it at what is in the way, it steps up what
    /// its step height allows, and its ground flag follows the move. It is not a teleport
    /// or a velocity setter: when the body simulates a collided move, the collision response
    /// can change its velocity, including through block restitution. The `entity_move` hook
    /// fires for it with `cause`, once, so
    /// a listener may replace or cancel the delta as for any other move; a cancelled move is
    /// `Some([0.0; 3])`. A body with no physics (a marker or `NoGravity` armor stand, or one a
    /// mod set) moves the whole delta with no collision. `MoveCause::Piston` also applies a
    /// piston's per-tick limit, 0.51 blocks per axis, on the first non-zero axis only; a
    /// mob's budget carries across calls in a tick, any other kind's lasts the call. Returns how far
    /// the entity moved, `delta` less what the collisions took. None for a stale handle, on a
    /// client, for an entity outside the game's entity walk (a display, a painting, an item
    /// frame, an End crystal, a dragon part) and for a delta that is not finite or has a
    /// component beyond 64 blocks. The base move's fall handling can remove a body before
    /// restitution. Block effects and triggers outside the base move still run at their own moments.
    ///
    /// A player moves as `Entity.move` with `MoverType.SELF` would, whatever `cause` the hook
    /// hears: blocks, entity and moving colliders and the player's collision exclusions stop
    /// it and a step up is taken (a spectator goes through everything, and `MoveCause::Piston`'s
    /// per-tick limit does not apply). The server's copy moves at once and the player's client
    /// is told with a relative teleport, so it adds the displacement to where it is when the
    /// packet arrives and keeps its motion and look. Until the client confirms, the server drops
    /// the player's own movement reports: a mod that moves a player on many ticks in a row holds
    /// it still (a development build warns after 20), so use `entity_push` or a client-side
    /// write for motion every tick. A player a moving collider carries stays carried while the
    /// target is still over the collider. `None` for a player who is a rider (its vehicle moves
    /// it), a sleeper, one without a connection, a target outside the world's bounds or a move
    /// the collision query cannot answer; nothing moves then.
    fn entity_move_by(
        &mut self,
        entity: EntityRef,
        delta: [f64; 3],
        cause: MoveCause,
    ) -> Option<[f64; 3]>;
    /// Side: server.
    /// Sends the entity to `pos` in the level `dimension` (`minecraft:the_nether`,
    /// or a level a mod registered), as `Entity.teleport` does with a portal's
    /// transition: `pos` is in that level's own coordinates (nothing scales it
    /// by `coordinate_scale`), a level nobody has entered opens first, and the
    /// entity keeps its motion as it is, unturned (`Relative.DELTA`; a
    /// minecart's is clamped to 0.4 along x and z, its `getKnownMovement`). A
    /// mod that wants it stopped or turned sets its velocity first
    /// (`entity_set_velocity`). It takes a player, a mob, a boat, a minecart (a
    /// vehicle takes its passengers along), an item, a primed TNT, an experience
    /// orb, an arrow, a trident, a firework and the thrown items and fireballs a
    /// chunk saves. Into another level anything but a player is recreated there
    /// with its stack, owner, thrower and portal cooldown, under a new handle:
    /// the old one is stale. That move is a change of level, not a section
    /// crossing: the section-crossing report (`entity_entering_section`) does not
    /// tell it, the entity's later moves there are told as any entity's, and this
    /// call adds no report of its own. A player arrives after the current tick,
    /// still (its client holds its motion), and `player_teleported` and
    /// `player_changed_dimension` report the move. `player_changed_dimension`
    /// (NeoForge's `PlayerChangedDimensionEvent`) is the point after which the
    /// player is in the target level, that level in place and the player's state
    /// in it: a mod's "entered" and "left" go by that hook, not by this call's
    /// return, when the player is still in the level it left. In the entity's own level it
    /// moves as `entity_teleport_to` does, keeping its motion. False for a stale
    /// handle, what `entity_teleport_to` refuses, a dimension that isn't
    /// registered, a primed TNT a mob rides, and the kinds it cannot move into
    /// another level (armor stands, falling blocks, ender pearls, wind charges,
    /// entities Sunrise doesn't simulate). A fake player stays where it is, and
    /// so does an entity whose travel a mod cancels (`entity_travel_to_dimension`);
    /// the call is still true. Nothing builds ground at the destination.
    fn entity_teleport_to_level(
        &mut self,
        entity: EntityRef,
        dimension: &str,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool;
    /// Side: server.
    /// Turns the entity to `yaw` and `pitch` in degrees (`Entity.setYRot`,
    /// `setXRot`): the pitch wraps and is clamped to -90..90. False for kinds
    /// whose rotation Sunrise does not fully store, or an unavailable player.
    /// Works for general entities; the new angles are tracked to clients.
    /// Does not teleport or dismount.
    fn entity_set_rotation(&mut self, entity: EntityRef, yaw: f32, pitch: f32) -> bool;
    /// Side: server.
    /// Turns a mob's head to `yaw` (`Entity.setYHeadRot`); its body follows
    /// as its AI turns it. Players' heads can turn too; false for other kinds
    /// or an unavailable player.
    fn entity_set_head_yaw(&mut self, entity: EntityRef, yaw: f32) -> bool;
    /// Side: server.
    /// Turns a painting, item frame or glow item frame in place to face `direction`
    /// (`HangingEntity.setDirection`, `ItemFrame.setDirection`): its facing, the yaw and pitch
    /// that follow from it and its bounds, nothing else. Like vanilla it doesn't check that the
    /// entity can still hang there: the next survival check does. A painting takes only a
    /// horizontal direction (vanilla's `Painting.setDirection` refuses up and down). Clients are
    /// told through the entity's data. False for another kind, a vertical direction for a
    /// painting, or a stale handle.
    fn entity_set_hanging_direction(&mut self, entity: EntityRef, direction: Direction) -> bool;
    /// Side: server.
    /// Turns the item in an item frame or glow item frame (`ItemFrame.setRotation`): `rotation`
    /// in eighths of a turn, wrapped to 0..=7 as vanilla's `rotation % 8`; the comparators reading
    /// the frame take its new signal. False for another kind or a stale handle.
    fn entity_set_item_frame_rotation(&mut self, entity: EntityRef, rotation: u8) -> bool;
}

/// The handles' client writes of the `entity_position` area, implemented by the game and reached through [`ClientBackendMut`].
// hidden: a handle's backend, reached through `ClientBackendMut`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityPositionClientBackendMut {
    /// Side: client.
    /// Moves or interpolates an observed remote entity to `position` (and
    /// optional `[yaw, pitch]` rotation in degrees), as vanilla's
    /// `Entity.moveOrInterpolateTo` does: routes to the entity family's own
    /// interpolation handler (its own step count), and falls back to setting
    /// the position and rotation directly for families without a handler (as
    /// vanilla's `InterpolationHandler.NO_OP` does).
    ///
    /// This updates client display state for bodies the server owns; the next
    /// server update wins. Returns `false` and changes nothing for a body the
    /// client simulates (the local player, a vehicle the local player is
    /// driving, or a `SERVER_AND_CLIENT` body such as an item, primed TNT,
    /// falling block, projectile, experience orb or dying dragon), for any
    /// non-finite coordinate or angle, for a stale or untracked handle, or
    /// outside a client world.
    fn entity_interpolate_to(
        &mut self,
        entity: EntityRef,
        position: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool;
}
