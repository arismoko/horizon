//! The `interaction_key_mapping_triggered` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A projectile hit something (NeoForge's `ProjectileImpactEvent`, posted
    /// where vanilla's projectile has found what it meets, before the hit
    /// acts): `projectile` (its entity id) of type `kind` (`minecraft:arrow`),
    /// shot by `owner` (a mob's or a player's entity id, 0 for none) in the level
    /// `dimension`. It hit the entity `entity` of type `entity_kind` (a player is
    /// `minecraft:player`; a dragon's part is `minecraft:ender_dragon`), or, with
    /// `entity` None, the block at `block` on its `face` (the face's normal,
    /// `[0, 1, 0]` for the top), at the point `at`. Set `canceled` and the impact
    /// does not happen: no damage, no knockback, no sound, no block reaction, the
    /// arrow does not stick, a pearl does not teleport, a potion or a fireball
    /// does not burst. Cancellation alone leaves the projectile alive. Resolve
    /// the live owner with `host.entity(event.projectile)` and call `EntityRef::discard`
    /// to consume it instead. Removal suppresses the hit even if the event is not
    /// canceled. A decision invalidated by listener writes is not recomputed.
    /// A flying bobber
    /// skips hooking and redirection to the hit point, but its ordinary move
    /// still stops against solid blocks. Water is not an impact. Bobbers notify
    /// before vanilla's entity deflection and world-border bounce; rockets notify
    /// after deflection and cancellation skips their impact explosion only.
    /// Covers arrows, tridents, thrown items, fireballs, wither skulls, wind
    /// charges, llama spit, shulker bullets, bobbers and rockets. Other families
    /// skip breeze deflections and hits on wind charges. A dragon part or End crystal is told by the id of the
    /// dragon or crystal. Server-side. Version 3 fires whenever a mod subscribes to
    /// `projectile_impact` directly (the separate `hears_projectile_impacts` opt-in
    /// poll hook is removed); version 2 added bobbers and rockets.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_projectile_impact",
        dispatch = "api_carrier",
        side = "server",
        version = 3,
        contract = "cancel",
        out = "canceled",
        neoforge = "ProjectileImpactEvent",
        cancellable = true
    )]
    fn projectile_impact(
        dimension: &str,
        projectile: u32,
        kind: &str,
        owner: u32,
        entity: Option<u32>,
        entity_kind: &str,
        block: [i32; 3],
        face: [i32; 3],
        at: [f64; 3],
        canceled: &mut bool,
    );
    /// An entity mounts or dismounts a vehicle (NeoForge's `EntityMountEvent`, posted from
    /// `Entity.startRiding` and `removeVehicle`): `entity` of type `kind` (`minecraft:player`,
    /// `minecraft:zombie`) in the server level `dimension`, getting on (`mounting`) or off the
    /// `vehicle` of type `vehicle_kind` (`minecraft:oak_boat`, `minecraft:minecart`,
    /// `minecraft:horse`, `minecraft:cushion`). Set `canceled` and it does not: a player stays
    /// where they were, a mob does not board, a `/ride` fails, and a rider whose dismount is
    /// canceled keeps riding. Players hear boarding and dismounts by input, `/ride`, hits,
    /// bucking, teleport or spectator mode; mobs hear boarding and dismounts (jockeys,
    /// ravager raiders, `/ride`, hits), including removal cleanup. The departing rider's own
    /// dismount is asked first, then its direct passengers in passenger-list order. The
    /// removed vehicle is no longer a valid handle or query result during these callbacks. A Mob's
    /// canceled dismount keeps the relationship with a removed boat, cart or mob; each later
    /// dismount asks again.
    /// A resident Mob retries immediately before its next eligible entity tick:
    /// its chunk must be entity-ticking and its tick must not have been canceled.
    /// A server player retries on its next vehicle tick. Either can be later in the
    /// same world tick. A rejected, nonresident passenger does not autonomously
    /// tick or retry. A mob riding a player is compulsorily ejected on save, logout,
    /// death or dimension change: the dismount notifies this hook, but cancellation
    /// cannot keep the mob aboard. Loading a save does not notify. A mob that boards a boat
    /// by touching it is checked again after the hook returns without canceling: the boat
    /// is found again by id (gone: the contact pass for that boat ends), its passengers are
    /// counted again, and the mob must still exist, not be removed and not already ride;
    /// a full boat or an unfit mob skips that mob. A player who sits on a cushion has the cushion
    /// found again by id after the board and dismount checks: gone, the click is spent and
    /// the player is not seated. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_mount",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityMountEvent",
        version = 2,
        cancellable = true
    )]
    fn entity_mount(
        dimension: &str,
        entity: u32,
        kind: &str,
        vehicle: u32,
        vehicle_kind: &str,
        mounting: bool,
        canceled: &mut bool,
    );
    /// An interaction mapping is about to act on the tick's crosshair target. `attempt`
    /// distinguishes a queued press from held use or mining. Use asks for each hand tried,
    /// main first. Cancelling the main hand ends the click. Attack and use honour
    /// `swing_hand` independently: it governs only the acting player's own screen.
    /// False suppresses natural local swings; true preserves natural no-swing outcomes.
    /// A cancelled use with true requests a local swing only: neither a use packet nor
    /// a swing packet is sent, so observers see no swing. An uncancelled use sends no
    /// extra client swing packet: observers see the server's success swing, regardless
    /// of this flag. Attack and mining swing packets also honour the item swing veto. Pick
    /// only vetoes its packet and never swings. Fires on the window client.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::interaction::ClientGame<'_>::native_interaction_key_mapping_triggered",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.InteractionKeyMappingTriggered",
        version = 2,
        cancellable = true
    )]
    fn interaction_key_mapping_triggered(
        key: InteractionKey,
        hand: Hand,
        attempt: InteractionAttempt,
        swing_hand: &mut bool,
        canceled: &mut bool,
    );
}
