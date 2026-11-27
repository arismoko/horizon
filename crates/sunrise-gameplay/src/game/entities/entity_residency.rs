//! One storage-family dispatch for the server's residency journal.
use super::commands::Target;
use mod_api::DimensionKey;
use sunrise_sim::join_seq::{
    Joined,
    residency::{
        Journal, Kind, Owner,
        table::{Location, SourceEntity},
    },
};

/// Assign a stored zero-id entity through its residency boundary exactly once.
macro_rules! assign_resident_id {
    ($list:expr, $index:expr, $entity:ident, $next:expr, $lifetime:expr) => {{
        let index = $index;
        if $list[index].id == 0 {
            let id = $list.update(index, |$entity| {
                $crate::game::entity_server::assign_stored_id($next, &mut $entity.id, $lifetime)
            });
            $list.id_assigned(index);
            id
        } else {
            $list[index].id
        }
    }};
}
pub(crate) use assign_resident_id;

macro_rules! assign_resident_ids {
    ($list:expr, $entity:ident, $next:expr, $lifetime:expr) => {
        for index in 0..$list.len() {
            $crate::game::entity_residency::assign_resident_id!(
                $list, index, $entity, $next, $lifetime
            );
        }
    };
}
pub(crate) use assign_resident_ids;

// Projectiles, knots and general rows have distinct Game/Parked field paths;
// keep binding and resolution on this one family list.
macro_rules! resident_owners {
    ($then:ident, $this:ident, $($projectiles:ident).+, $($knots:ident).+, $($general:ident).+, $($others:tt).+) => {
        $then! {
            ($this, $($projectiles).+)
            Mob => Mob: mobs,
            Item => Item: dropped,
            Orb => Orb: orbs,
            Stand => Stand: armor_stands,
            Frame => Frame: item_frames,
            Painting => Painting: paintings,
            Boat => Boat: boats,
            Cart => Cart: carts,
            Other => Other: $($others).+,
            Fangs => Fangs: fangs.list,
            Crystal => Crystal: crystals,
            Tnt => Tnt: tnts,
            Cushion => Cushion: cushions,
            Knot => Knot: $($knots).+,
            Falling => Falling: falling,
            Rocket => Rocket: rockets,
            Trident => Trident: tridents,
            Spawner => Spawner: ominous_spawners,
            Cloud => Cloud: breath_clouds,
            Bolt => Bolt: bolts,
            General => General: $($general).+,
        }
    };
}
pub(crate) use resident_owners;

macro_rules! resident_visible {
    (Mob, $entity:expr) => {
        !$entity.removed
    };
    ($kind:ident, $entity:expr) => {
        true
    };
}
pub(crate) use resident_visible;

/// Physical lifetime access, not a cached publication or a delivery promise.
#[derive(Debug, PartialEq, Eq)]
pub enum SlotAccess<R> {
    Done(R),
    Taken,
    Missing,
    UnsupportedKind(Kind),
}

macro_rules! resident_lifetime_slot {
    ($kind:ident, $list:expr, $join:expr, $visit:expr) => {{
        let Some(index) = $list.locate($join) else {
            return SlotAccess::Missing;
        };
        SlotAccess::Done($list.update(index, |entity| {
            $visit(
                sunrise_mod_runtime::attachments::HasAttachmentLifetime::attachment_lifetime(
                    entity,
                ),
            )
        }))
    }};
}
pub(crate) use resident_lifetime_slot;

macro_rules! define_resident_access {
    (($this:ident, $($projectiles:ident).+) $($kind:ident => $target:ident: $($field:tt).+,)*) => {
        pub(in crate::game::server_side) fn bind_resident_owners(&mut $this, journal: &Journal, dimension: DimensionKey, track_saved_uuids: bool) {
            if track_saved_uuids {
                $this.persist.uuids.bind(journal, dimension);
            }
            $(sunrise_gameplay_common::game::resident_state::bind_resident_list!($kind, $this$(.$field)+, journal, Owner { dimension, kind: Kind::$kind });)*
            $this$(.$projectiles)+.bind_residency(journal, Owner { dimension, kind: Kind::Projectile }, |entity| (entity.id, entity.join_seq()));
            $this.dragon.bind_residency_specialized(journal, Owner { dimension, kind: Kind::Dragon }, |entity| (entity.id, entity.join_seq()), sunrise_sim::join_seq::residency::Source::Ordinary, Some((Kind::DragonPart, 8, |dragon, offset| dragon.parts.get(usize::from(offset)).map(|part| part.3))), None);
        }
        pub(in crate::game::server_side) fn with_resident_owner_slot<R>(
            &mut $this,
            at: Location,
            visit: impl FnOnce(&mut sunrise_mod_runtime::attachments::AttachmentLifetime) -> R,
        ) -> $crate::game::entity_residency::SlotAccess<R> {
            use $crate::game::entity_residency::SlotAccess;
            match at.owner.kind {
                $(Kind::$kind => $crate::game::entity_residency::resident_lifetime_slot!($kind, $this$(.$field)+, at.join, visit),)*
                Kind::Projectile => $crate::game::entity_residency::resident_lifetime_slot!(Projectile, $this$(.$projectiles)+, at.join, visit),
                Kind::Dragon => {
                    $this.dragon.locate(at.join).map_or(SlotAccess::Missing, |_| {
                        use sunrise_mod_runtime::attachments::HasAttachmentLifetime;
                        SlotAccess::Done(visit($this.dragon.as_mut().unwrap().attachment_lifetime()))
                    })
                },
                Kind::DragonPart => {
                    use sunrise_sim::join_seq::residency::Source;
                    use sunrise_mod_runtime::attachments::HasAttachmentLifetime;
                    let Source::DragonPart(offset) = at.source else { return SlotAccess::Missing; };
                    if $this.dragon.locate(at.join).is_none() { return SlotAccess::Missing; }
                    let Some(part) = $this.dragon.as_mut().and_then(|dragon| dragon.parts.get_mut(usize::from(offset))) else { return SlotAccess::Missing; };
                    SlotAccess::Done(visit(part.attachment_lifetime()))
                },
                Kind::Player | Kind::Hook => SlotAccess::UnsupportedKind(at.owner.kind),
            }
        }
        pub(in crate::game::server_side) fn locate_resident_attachment_owner(&$this, at: Location) -> Option<SourceEntity<(Target, bool)>> {
            let source = match at.owner.kind {
                $(Kind::$kind => {
                    let index = $this$(.$field)+.locate(at.join)?;
                    let entity = &$this$(.$field)+[index];
                    let visible = $crate::game::entity_residency::resident_visible!($kind, entity);
                    let id = entity.id;
                    Some(SourceEntity { id, value: (Target::$target(id), visible) })
                })*
                Kind::Projectile => {
                    let index = $this$(.$projectiles)+.locate(at.join)?;
                    let id = $this$(.$projectiles)+[index].id;
                    Some(SourceEntity { id, value: (Target::Projectile(id), true) })
                }
                Kind::Dragon => {
                    $this.dragon.locate(at.join)?;
                    let id = $this.dragon.as_ref()?.id;
                    Some(SourceEntity { id, value: (Target::Dragon(id), true) })
                }
                Kind::DragonPart => {
                    let sunrise_sim::join_seq::residency::Source::DragonPart(offset) = at.source else { return None; };
                    $this.dragon.locate(at.join)?;
                    let dragon = $this.dragon.as_ref()?;
                    if dragon.id == 0 { return None; }
                    dragon.parts.get(usize::from(offset))?;
                    let id = sunrise_gameplay_common::game::dragon_common::dragon_part_id(dragon.id, usize::from(offset));
                    let visible = true;
                    Some(SourceEntity { id, value: (Target::DragonPart(id), visible) })
                }
                Kind::Player | Kind::Hook => None,
            };
            source
        }
        pub(in crate::game::server_side) fn locate_resident_owner(&$this, at: Location) -> Option<SourceEntity<(Target, bool)>> {
            let source = $this.locate_resident_attachment_owner(at).map(|mut source| {
                if let Target::DragonPart(_) = source.value.0 {
                    source.value.1 &= $this.dragon.as_ref().is_some_and(|dragon| !$this.persist.removals.contains(&Target::Dragon(dragon.id)));
                }
                source
            });
            // Logical visibility is not physical absence: filtering here would
            // incorrectly evict reversible conversion rows or report Taken.
            source.map(|mut source| {
                source.value.1 &= !$this.persist.removals.contains(&source.value.0);
                source.value.1 &= !$this.persist.destroying_vehicles.contains(&source.value.0);
                source
            })
        }
    };
}
pub(crate) use define_resident_access;

impl crate::game::ServerGame<'_> {
    resident_owners!(
        define_resident_access,
        self,
        creatures.projectiles,
        resident_entities.knots,
        resident_entities.general.rows,
        server.others.0
    );
}
