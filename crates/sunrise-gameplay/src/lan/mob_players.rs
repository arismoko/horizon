//! Which player a mob sees in an open world, and how the blocks that look for players see a guest.
use sunrise_core::geometry::{Aabb, Vec3};
pub use sunrise_gameplay_common::lan::mob_players::{GuestViews, acts_on_player};
use sunrise_sim::mobs::{Mob, PlayerView, Species, Who};

/// The player a mob sees this tick: `None` for the host, else a guest.
/// A mob's target is one player (`Mob.getTarget` is an entity): while it has
/// one, it keeps that player, host or guest, as long as they are in play, and
/// its goals drop a target that stopped being valid; it never passes to
/// another player. A mob that a guest hurt in the last 100 ticks
/// (`LivingEntity.getLastHurtByMob`), or an angry golem, keeps that guest while
/// they are still a valid target; otherwise it sees the nearest player
/// (`TargetingConditions`: alive, neither creative nor spectator), or the
/// nearest one that is not a spectator if none qualifies.
pub fn player_for(
    m: &Mob,
    host: Option<&PlayerView>,
    guests: &GuestViews,
    bound: Option<u32>,
) -> Option<u32> {
    // A tame cat reads its owner wherever they are while they sleep, or just
    // woke up (`CatRelaxOnOwnerGoal` uses `getOwner`, not the nearest player).
    if m.species() == Species::Cat
        && m.extra.tame
        && let Some(owner) = m.extra.owner
    {
        let asleep = |v: &PlayerView| v.uuid == owner && (v.sleeping || v.sleep_timer > 0);
        if host.is_some_and(asleep) {
            return None;
        }
        if let Some((conn, _)) = guests.iter().find(|(_, v)| asleep(v)) {
            return Some(*conn);
        }
    }
    // An iron golem's anger is for the player who hit it (`NeutralMob` keeps
    // the angry-at UUID), and outlasts the 100 ticks of `getLastHurtByMob`:
    // while it lasts, the golem keeps that player, not whoever is nearest.
    let angry = m.species() == Species::IronGolem && m.extra.angry_at == Some(Who::Player);
    let targeting = m.ai.target == Some(Who::Player)
        || angry
        || m.ai.last_hurt_by == Some(Who::Player) && m.age as i32 - m.ai.last_hurt_by_time <= 100;
    // A tame wolf, cat or parrot sees its owner (`TamableAnimal.getOwner`), while they
    // are in play, whoever is nearer. Not while it targets or was just hurt by
    // a player: then `Who::Player` is that attacker, whoever it is, and the
    // owner goals ignore it until the wolf lets go.
    if !targeting
        && matches!(m.species(), Species::Wolf | Species::Cat | Species::Parrot)
        && m.extra.tame
        && let Some(owner) = m.extra.owner
    {
        if host.is_some_and(|v| v.alive && v.uuid == owner) {
            return None;
        }
        if let Some((conn, _)) = guests.iter().find(|(_, v)| v.alive && v.uuid == owner) {
            return Some(*conn);
        }
    }
    // A mob on a player's lead follows and measures its rope against the
    // player who holds it, not whoever is nearest or in survival mode.
    if m.extra.leash == Some(sunrise_sim::mobs::leash::Holder::Player)
        && let Some(holder) = m.extra.leash_player
    {
        if host.is_some_and(|v| v.uuid == holder) {
            return None;
        }
        if let Some((conn, _)) = guests.iter().find(|(_, v)| v.uuid == holder) {
            return Some(*conn);
        }
    }
    let aimed = m.ai.target == Some(Who::Player);
    choose(m.position, targeting, aimed, host, guests, bound)
}

pub fn choose(
    at: Vec3,
    targeting: bool,
    aimed: bool,
    host: Option<&PlayerView>,
    guests: &GuestViews,
    bound: Option<u32>,
) -> Option<u32> {
    let targetable = |v: &PlayerView| v.alive && !v.creative && !v.spectator;
    if aimed {
        match bound {
            Some(conn) if guests.iter().any(|(c, _)| *c == conn) => return Some(conn),
            None if host.is_some() => return None,
            _ => {}
        }
    }
    if targeting
        && let Some(conn) = bound
        && guests.iter().any(|(c, v)| *c == conn && targetable(v))
    {
        return Some(conn);
    }
    let d = |v: &PlayerView| at.distance_squared(v.position);
    let nearest = |ok: &dyn Fn(&PlayerView) -> bool| {
        let mut best = host.filter(|v| ok(v)).map(|v| (None, d(v)));
        for (conn, v) in guests.iter().filter(|(_, v)| ok(v)) {
            if best.is_none_or(|(_, b)| d(v) < b) {
                best = Some((Some(*conn), d(v)));
            }
        }
        best
    };
    nearest(&targetable)
        .or_else(|| nearest(&|v: &PlayerView| v.alive && !v.spectator))
        .and_then(|(conn, _)| conn)
}

/// A guest's `player_view`, taken with their state swapped in, as the mobs see
/// them. Their last attacker and last victim stay: a tamed wolf defends and
/// helps whoever owns it (`OwnerHurtByTargetGoal`, `OwnerHurtTargetGoal`), and
/// the window's own player is a guest of its server thread.
pub fn guest_view(mut v: PlayerView, uuid: u128, moved: [f64; 3]) -> PlayerView {
    v.hero = false;
    v.velocity = Vec3::new(moved[0], moved[1], moved[2]);
    v.uuid = sunrise_sim::lan::playerdata::uuid_ints(uuid);
    v.allies = Default::default();
    v.remote = true;
    v
}

/// A guest as the blocks that look for players see them
/// (`PlayerDetector`, `level.getPlayers`: the dead ones too).
pub struct Present {
    pub uuid: [i32; 4],
    pub position: [f64; 3],
    pub eye: [f64; 3],
    pub height: f32,
    pub creative: bool,
    pub spectator: bool,
    pub alive: bool,
    /// Trial Omen, else Bad Omen, among the guest's effects.
    pub omen: Option<sunrise_sim::mobs::trial_spawner::Omen>,
}

/// `TrialSpawner.findPlayerWithOminousEffect`: Trial Omen first, then Bad Omen.
pub fn omen_of(
    effects: &sunrise_level::effects::PlayerEffects,
) -> Option<sunrise_sim::mobs::trial_spawner::Omen> {
    use sunrise_sim::mobs::trial_spawner::Omen;
    if effects.get("trial_omen").is_some() {
        Some(Omen::Trial)
    } else {
        effects.get("bad_omen").map(|_| Omen::Bad)
    }
}

/// A guest's bounding box, by its pose (`Avatar.POSES`).
pub fn guest_bounds(g: &sunrise_sim::lan::guest::Guest) -> Aabb {
    g.bounds()
}
