//! Fence knot state and the pure leash decisions of leads.
use sunrise_core::nbt::Tag;
use sunrise_sim::mobs::leash::{self, Holder, Pending};

/// Natural fence knot: the fence is its authoritative position, not a render cache.
/// Its JoinList carries the common residency binding and journal.
pub struct Knot {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub id: u32,
    pub join: sunrise_sim::join_seq::JoinSeq,
    pub fence: [i32; 3],
    /// Runtime fire ticks use NBT's signed short only when persisted.
    pub fire_ticks: i32,
    pub invulnerable: bool,
    pub no_gravity: bool,
    pub silent: bool,
    pub glowing: bool,
}
sunrise_sim::impl_joined!(Knot);
sunrise_mod_runtime::impl_attachment_lifetime!(Knot);

/// Whether a mob's lead is held by `acting` (`getLeashHolder() == player`).
pub fn held_by_acting_player(
    leash: Option<Holder>,
    leash_player: Option<[i32; 4]>,
    acting: [i32; 4],
) -> bool {
    leash == Some(Holder::Player) && leash_player.is_none_or(|p| p == acting)
}

/// Resolve a `Pending` leash holder loaded from NBT.
pub fn resolve_pending_leash(
    pending: Pending,
    player_uuid: [i32; 4],
    owner_uuid: Option<[i32; 4]>,
    is_guest: impl FnOnce([i32; 4]) -> bool,
    mob_by_uuid: impl FnOnce([i32; 4]) -> Option<u32>,
    boat_by_uuid: impl FnOnce([i32; 4]) -> Option<u32>,
    knot_by_uuid: impl FnOnce([i32; 4]) -> Option<[i32; 3]>,
) -> Option<(Holder, Option<[i32; 4]>)> {
    match pending {
        Pending::Knot(p) => Some((Holder::Knot(p), None)),
        Pending::Uuid(u) if is_guest(u) || owner_uuid == Some(u) => Some((Holder::Player, Some(u))),
        // Old integrated worlds wrote offline_uuid("Player") for every holder.
        // Only that known sentinel can be mapped to the integrated owner; a
        // different level player UUID may belong to somebody else. Old guest
        // leashes lost their real UUID, so this owner recovery is best effort.
        Pending::Uuid(u)
            if u == player_uuid
                && u == sunrise_sim::lan::playerdata::uuid_ints(
                    sunrise_protocol::crypto::offline_uuid("Player"),
                )
                && owner_uuid.is_some() =>
        {
            Some((Holder::Player, owner_uuid))
        }
        // `Leashable.restoreLeashFromSave`: `ServerLevel.getEntity(uuid)`, a
        // leash knot included.
        Pending::Uuid(u) => mob_by_uuid(u)
            .map(|id| (Holder::Mob(id), None))
            .or_else(|| boat_by_uuid(u).map(|id| (Holder::Boat(id), None)))
            .or_else(|| knot_by_uuid(u).map(|p| (Holder::Knot(p), None))),
    }
}

/// Write a mob's `leash` NBT entry into `tag`.
pub fn write_mob_leash_tag(
    tag: &mut Tag,
    holder: Option<Holder>,
    pending: Option<Pending>,
    leash_player: Option<[i32; 4]>,
    level_player_uuid: Option<[i32; 4]>,
    mob_uuid: impl FnOnce(u32) -> [i32; 4],
) {
    let Tag::Compound(map) = tag else { return };
    map.remove(&"leash".into());
    let uuid = match (holder, pending) {
        (Some(Holder::Player), _) => leash_player.or(level_player_uuid).or(Some([0, 0, 0, 1])),
        (Some(Holder::Mob(id) | Holder::Boat(id)), _) => Some(mob_uuid(id)),
        (_, Some(Pending::Uuid(u))) => Some(u),
        _ => None,
    };
    let value = match (holder, pending) {
        (Some(h), _) => leash::to_nbt(h, uuid),
        (None, Some(Pending::Knot(p))) => leash::to_nbt(Holder::Knot(p), None),
        (None, Some(Pending::Uuid(u))) => leash::to_nbt(Holder::Player, Some(u)),
        _ => None,
    };
    if let Some(v) = value {
        map.insert("leash".into(), v);
    }
}
