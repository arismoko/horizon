//! The `block_landing` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity landed on a block state this part is attached to (`Block.fallOn`), after falling `event.distance` blocks. Writable `event.distance` and `event.damage_multiplier` feed the fall calculation before hurt and sounds. The multiplier starts at the block's vanilla value (normally 1.0). Damage is `floor((distance + 1e-6 - safe_fall_distance) * damage_multiplier * fall_damage_multiplier_attribute)`, with safe fall distance 3 by default. Halving distance is not halving damage. Zero multiplier or a distance within the safe fall prevents damage and generic fall sounds. An entity that cannot be hurt by the fall (creative, fall immune) is told with zero multiplier and keeps its immunity, and a cancelled fall still lands. It runs inside the entity's move, on the level random host: it reads blocks and draws the level's random in vanilla's order, and cannot write. A write the override makes (farmland turning to dirt, a turtle egg breaking) is asked with `event.request(LandingWrite::SetBlock(state))` or `LandingWrite::DestroyBlock { drop }` and lands at the block the entity landed on once the entity's batch has ticked, in the order asked, if the block is still the state it landed on. SetBlock takes an API BlockState. The complete requested batch is checked before queuing: an invalid key or ordinal refuses all writes and sibling outputs from that callback, including distance and multiplier. The original landed-state guard is checked once per batch, not between its ordered writes. Players, mobs and dropped items land here (an item as an `EntityRef` of kind `minecraft:item`, with zero multiplier: items take none); projectiles and the other entities do not.
    #[hook(
        side = "server",
        contract = "collect",
        out = "damage_multiplier",
        scope = "block",
        key = "state",
        host = "level",
        target = "sunrise_gameplay_common::game::native_hooks::block_landing::HitLevel<'_>::native_fall_on",
        neoforge = "LivingFallEvent",
        version = 2
    )]
    fn fall_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        distance: &mut f64,
        damage_multiplier: &mut f32,
        writes: &mut Vec<LandingWrite>,
    );
    /// An entity bounced off a block state this part is attached to (`Block.bounceOn`). `event.fall_distance` is the distance stored after fall handling, so a grounded boat can report zero. It fires only when the entity bounced, just before vanilla sets its movement. `event.movement` is the movement the bounce leaves it (`x`, `y`, `z`): change it to change what the entity keeps. It runs inside the entity's move, on the level random host: it reads blocks and draws the level's random in vanilla's order, but writes nothing. Runs on the server for players, mobs and other bodies that bounce in the base move.
    #[hook(
        side = "server",
        contract = "collect",
        out = "movement",
        scope = "block",
        key = "state",
        host = "level",
        target = "sunrise_gameplay_common::game::native_hooks::block_landing::HitLevel<'_>::native_bounce_on",
        neoforge = "none (vanilla Block.bounceOn; IBlockExtension.getBounceRestitution only returns a factor)",
        version = 2
    )]
    fn bounce_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        fall_distance: f64,
        movement: &mut [f64; 3],
    );
}
