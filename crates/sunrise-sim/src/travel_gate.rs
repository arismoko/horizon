//! The living travel gate (#2414 PR 2): the door the `living_travel` hook
//! fires through, at the two boundaries the maintainer ruling names - the
//! player's `Player.travel` branch selection (`MotionWorld::player_travel`)
//! and the mob's `LivingEntity.travel` entry (`Mob::travel`). A travel that
//! runs there asks this gate first; with no listener, or on a client's own
//! prediction, the resolved input and velocity stand and the tick cost is
//! one branch.
use mod_api::TravelMode;
use sunrise_core::geometry::Vec3;
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::travel::TravelMode as GameTravelMode;

/// The `living_travel` hook's target (#2414): the generated dispatch the
/// catalog's generator requires of a target, with the loop's own reader as
/// the context. The travel gate uses the same generated dispatch.
pub fn native_living_travel(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    mode: TravelMode,
    input: &mut [f64; 3],
    velocity: &mut [f64; 3],
    canceled: &mut bool,
) {
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_living_travel(
        &reader.context(),
        entity,
        kind,
        mode,
        input,
        velocity,
        canceled,
    );
}

/// The `living_travel_moved` hook's target (#5074): the generated dispatch the
/// catalog's generator requires of a target, with the loop's own reader as the
/// context. The moved gate uses the same generated dispatch.
pub fn native_living_travel_moved(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    mode: TravelMode,
    moved: [f64; 3],
    on_ground_before: bool,
    on_ground: bool,
    jumping: bool,
    velocity: &mut [f64; 3],
) {
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_living_travel_moved(
        &reader.context(),
        entity,
        kind,
        mode,
        moved,
        on_ground_before,
        on_ground,
        jumping,
        velocity,
    );
}

/// The mode the mod API names for a game-side [`GameTravelMode`]
/// (`living_travel` of #2414): the game's own branch names, one for one.
fn mode_of(mode: GameTravelMode) -> TravelMode {
    match mode {
        GameTravelMode::Air => TravelMode::Air,
        GameTravelMode::Water => TravelMode::Water,
        GameTravelMode::Lava => TravelMode::Lava,
        GameTravelMode::Glide => TravelMode::Glide,
        GameTravelMode::Flying => TravelMode::Flying,
        GameTravelMode::VelocityOnly => TravelMode::VelocityOnly,
    }
}

/// Ask one travel invocation (#2414) about to run for `entity` of `kind` in
/// `mode`'s branch: the `living_travel` hook fires here - in this one
/// function - on whichever side runs the travel (the server's, and the
/// client's own prediction), and only when a listener subscribes; with none
/// the resolved values stand and the cost is the subscriber branch. `mode` names the
/// branch only when a listener asks for it. A listener that cancels skips
/// the invocation - its nested move does not run and the flying vertical
/// damping is skipped; what the tick already did is not undone. Listeners
/// registered with `#[listener(receive_cancelled)]` may clear cancellation.
/// Replacements pass through to `Entity.moveRelative` and
/// `Entity.setDeltaMovement` without synthetic limits or output validation
/// during dispatch. If a listener unwinds, only its own edits are discarded
/// and that callback is disabled for this hook for the session. Earlier edits
/// stand and later eligible listeners still run.
///
/// Returns whether the travel runs; `input` and `velocity` are the values
/// the branch starts from.
#[inline(never)]
pub fn decide(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    mode: impl FnOnce() -> GameTravelMode,
    input: &mut Vec3,
    velocity: &mut Vec3,
) -> bool {
    decide_controlled(context, entity, kind, mode, input, velocity, false)
}

/// [`decide`] with an explicit `client_controlled` flag for a body a client
/// controls (a player, or a vehicle a player is driving): `side = "server"`
/// listeners are skipped for those bodies.
#[inline(never)]
pub fn decide_controlled(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    mode: impl FnOnce() -> GameTravelMode,
    input: &mut Vec3,
    velocity: &mut Vec3,
    client_controlled: bool,
) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::LivingTravel) {
        return true;
    }
    let client_controlled = client_controlled || kind == "minecraft:player";
    let mode = mode_of(mode());
    let mut replaced_input = [input.x, input.y, input.z];
    let mut replaced_velocity = [velocity.x, velocity.y, velocity.z];
    let mut canceled = false;
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_living_travel_for(
        &reader.context(),
        entity,
        kind,
        mode,
        &mut replaced_input,
        &mut replaced_velocity,
        &mut canceled,
        client_controlled,
    );
    if canceled {
        return false;
    }
    *input = Vec3::new(replaced_input[0], replaced_input[1], replaced_input[2]);
    *velocity = Vec3::new(
        replaced_velocity[0],
        replaced_velocity[1],
        replaced_velocity[2],
    );
    true
}

/// What one travel branch's own `Entity.move(SELF)` left, beyond the velocity: the
/// inputs `living_travel_moved` reports (#5074).
#[derive(Clone, Copy, Debug)]
pub struct Moved {
    /// The displacement the move made; zero when `entity_move` vetoed it.
    pub moved: Vec3,
    /// The ground flag the body entered the move with.
    pub on_ground_before: bool,
    /// The ground flag after the move.
    pub on_ground: bool,
    /// The body's jump input this tick.
    pub jumping: bool,
}

/// Fire `living_travel_moved` (movement phase 4, #5074) for one travel
/// invocation of `entity` of `kind` that has just run its branch's own
/// `Entity.move(SELF)` and not yet its drag and gravity: on whichever side
/// simulates the body (the server's, and a client's own prediction), and only
/// when a listener subscribes; with none the velocity stands and the cost is
/// the subscriber branch. `mode` names the branch only when a listener asks
/// for it. A finite replacement becomes the velocity the branch's drag and
/// gravity act on; a non-finite one is dropped for that listener alone, and a
/// listener that unwinds loses only its own edit.
///
/// `client_controlled` is true for a body a client controls (a player, or a
/// vehicle a player is driving): a `side = "server"` listener is skipped for
/// those bodies.
#[inline(never)]
pub fn moved_controlled(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    mode: impl FnOnce() -> GameTravelMode,
    after: Moved,
    velocity: &mut Vec3,
    client_controlled: bool,
) {
    if !mod_api::__private::listened(mod_api::HookId::LivingTravelMoved) {
        return;
    }
    let client_controlled = client_controlled || kind == "minecraft:player";
    let mut replaced = [velocity.x, velocity.y, velocity.z];
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_living_travel_moved_for(
        &reader.context(),
        entity,
        kind,
        mode_of(mode()),
        [after.moved.x, after.moved.y, after.moved.z],
        after.on_ground_before,
        after.on_ground,
        after.jumping,
        &mut replaced,
        client_controlled,
    );
    *velocity = Vec3::new(replaced[0], replaced[1], replaced[2]);
}
