//! The `pickup_xp` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before `player` picks up `orb` in `dimension` (`ExperienceOrb.playerTouch`,
    /// NeoForge's `PlayerXpEvent.PickupXp`). Fires only when `takeXpDelay` is 0.
    /// `orb` is the live [`ExperienceOrbRef`] handle—there is no separate snapshot
    /// copy, so reads (`value`, `count`, `age`) and writes (`set_value`, `set_count`,
    /// `set_age`, or `orb.entity().kill(host)`) act on the live orb immediately and
    /// later listeners observe earlier edits. `set_value` keeps vanilla's signed
    /// `int` (`DATA_VALUE`) semantics (zero and negative values repair nothing and
    /// award no XP; a surviving orb persists `Value` as a signed short); `set_count`
    /// enforces vanilla's `ExtraCodecs.POSITIVE_INT` (`1..=i32::MAX`) and returns
    /// `Err(ExperienceOrbWriteError::InvalidCount)` without mutating the orb otherwise.
    /// Canceling (`Flow::Cancel`) skips the pickup continuation (`takeXpDelay` stays
    /// 0, no take/sound, no Mending/XP award, and no count decrement), leaving any
    /// writes already made through `orb` in place. Otherwise, if `orb` is still
    /// resident, pickup runs in vanilla `ExperienceOrb.playerTouch` order: sets
    /// `takeXpDelay` to 2, takes the orb, repairs Mending and awards remaining
    /// positive XP, then decrements `count` and discards at 0.
    /// Server-side. Diverges from upstream: renamed from `experience_orb_pickup`, to
    /// NeoForge's `PlayerXpEvent.PickupXp`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_pickup_xp",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerXpEvent.PickupXp",
        cancellable = true,
        version = 2
    )]
    fn pickup_xp(player: u128, dimension: &str, orb: ExperienceOrbRef, canceled: &mut bool);
}
