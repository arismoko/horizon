//! The `mob_despawn` hook: declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `MobDespawnEvent`: the mob `mob` (of type `kind`, at `pos`) is about to run `Mob.checkDespawn`, which fires once per tick for every mob on the entity tick list (its chunk is entity-ticking, which player tickets give across the whole view distance), persistent ones and peaceful-mode ones included. A listed mob beyond the simulation distance is checked without being ticked; a mob in a loaded chunk that is not entity-ticking is neither checked nor told. The hook is told while the game has lent its mob list to the tick, so `mob` resolves through the tick's own read view (#795): the ticking mob itself, and the other present mobs in its tick snapshot, are readable through the host. The ticking mob is read directly; an already-ticked peer exposes its post-turn state and a not-yet-ticked peer its pre-turn state. A mob spawned during the pass does not resolve in this scoped view until the next pass. Removed mobs do not resolve; a dead-but-present mob is not a removed mob. This is the scoped read-host first step for #795, not an in-place ownership redesign. `kind` and `pos` stay as plain data for listeners that only need them. Set `result`: `Allow` discards the mob now, `Deny` keeps it this tick, and `Default` (the starting value) leaves vanilla's rules to decide. Anything but `Default` skips vanilla's checks, including the `nextInt(800)` draw of the idle-despawn roll, so the hook fires before that draw: a mob a mod denies does not consume it. A listener is a pure function of its inputs, because the hook fires on replayed ticks. The wither and the ender dragon override `checkDespawn` without `super`, so the event is not fired for them. Server-side. Tick-time effect: one `listened` branch per mob per tick with no listeners.
    #[notes = r#"## Mob despawn

- **`mob_despawn(dimension, mob, kind, pos, result)`** (NeoForge's `MobDespawnEvent`, server): fired once per tick for every mob on the entity tick list just before `Mob.checkDespawn`, including persistent mobs and mobs in peaceful. A mob is on the list when its chunk is entity-ticking, which player tickets give across the whole view distance; listed mobs beyond the simulation distance are checked without being ticked. `result` starts as `Despawn::Default`. `Allow` discards the mob now, `Deny` keeps it this tick (a mod can keep a hostile mob in peaceful, as NeoForge's event documents), and `Default` leaves vanilla's rules to decide. An answer other than `Default` skips vanilla's checks, so it also skips the `nextInt(800)` draw of the idle-despawn roll: the hook fires before that draw. A mob in a loaded chunk that is not entity-ticking is not checked and the hook does not fire for it; the wither and the ender dragon override `checkDespawn` without `super`, so it does not fire for them either. In vanilla the whole of `Mob.checkDespawn` follows the event, the peaceful-mode discard included, so a `Deny` keeps a hostile mob in peaceful.
- Version 2 makes the ticking mob and snapshot peers readable through the scoped tick host. The ticking mob is read directly; an already-ticked peer exposes post-turn state and a not-yet-ticked peer pre-turn state. A mob spawned during the pass does not resolve in this view until the next pass. Removed mobs do not resolve, unlike dead-but-present mobs.
- Version 3 narrows the hook to the entity tick list: mobs in loaded chunks that are not entity-ticking no longer fire it or check despawning, as in vanilla.
- Listeners run on replayed ticks, so they are pure functions of their inputs. Tick-time effect: one `listened` branch per mob per tick with no listeners; the answer sits in a `Mob` field that costs one byte and is reset after the mob's tick.
- Beyond NeoForge: none. `Despawn` is plain data, `#[non_exhaustive]`."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        version = 3,
        contract = "listen",
        host = "read",
        target = "sunrise_gameplay_common::game::native_hooks::living_change_target::NativeHookHost<'_>::native_mob_despawn",
        neoforge = "MobDespawnEvent",
        cancellable = false
    )]
    fn mob_despawn(dimension: &str, mob: MobRef, kind: &str, pos: [f64; 3], result: &mut Despawn);
}
