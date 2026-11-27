//! The one rule for which entities are solid to a mover (`EntityGetter.getEntityCollisions`):
//! vanilla's `Entity.canBeCollidedWith`, `Entity.canCollideWith` and
//! `AbstractBoat.canVehicleCollide`, shared by a mob's move (`mobs/living.rs`) and every other
//! mover (`game/entities/collision_source.rs`). The callers own their inputs (the entity lists,
//! a spatial query, a prebuilt grid) and ask about each candidate here.
use sunrise_core::geometry::Aabb;

/// What a candidate is, as far as `canBeCollidedWith` tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidKind {
    /// `Shulker.canBeCollidedWith`: while alive.
    Shulker,
    /// `HappyGhast.canBeCollidedWith`: an adult, while held still.
    HappyGhast,
    Boat,
    Cart,
    /// A player, offered only when it may block (not a spectator, not dead, not riding the mover).
    Player,
    /// Every other mob, which keeps `Entity`'s `false` unless a vehicle pushes it.
    Mob,
}

/// One entity a caller offers as a possible obstacle.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub id: u32,
    pub kind: SolidKind,
    pub bounds: Aabb,
    pub alive: bool,
    pub removed: bool,
    pub baby: bool,
    /// A happy ghast's `isOnStillTimeout`.
    pub still: bool,
    /// Something rides it.
    pub carrying: bool,
    /// `Entity.isPushable`, without the shulker's exception `canVehicleCollide` adds.
    pub pushable: bool,
    /// What it rides directly.
    pub mount: Option<u32>,
}

impl Candidate {
    /// A live entity that is none of the mobs' states: a boat, a cart or a player.
    pub fn fixed(kind: SolidKind, id: u32, bounds: Aabb) -> Self {
        Self {
            id,
            kind,
            bounds,
            alive: true,
            removed: false,
            baby: false,
            still: false,
            carrying: false,
            pushable: false,
            mount: None,
        }
    }
}

/// Which side of the connection runs the rule: `Level.isClientSide()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Client,
    Server,
}

/// Who is moving, which decides the rule.
#[derive(Clone, Copy, Debug)]
pub enum Mover {
    /// A mob: a living shulker, or a happy ghast held still (or one carrying someone, when the
    /// mover is a happy ghast), except itself, its vehicle and its fellow passengers.
    Mob {
        id: u32,
        happy_ghast: bool,
        mount: Option<u32>,
    },
    /// The player: a boat or a happy ghast it does not ride, a living shulker; on the client a
    /// ghast also when the player's feet are over its top (`HappyGhast.canBeCollidedWith`'s
    /// `isClientSide` branch, its predictive landing), on the server only one held still.
    Player {
        feet_y: f64,
        riding: Option<u32>,
        side: Side,
    },
    /// A dropped item, an armor stand, an orb, primed TNT, a falling block, a pushed body:
    /// a boat, a living shulker, a happy ghast held still.
    Passive,
    /// `AbstractBoat.canVehicleCollide` for a cart: the others, every living pushable mob, a
    /// shulker, every player offered; not a mob riding this cart.
    Cart { id: u32 },
    /// A boat excludes its whole riding stack, not just its own passengers.
    Boat { id: u32, root: Option<u32> },
}

impl Mover {
    /// Whether any candidate of `kind` can be solid to this mover: a cheap test a caller makes
    /// before it works a candidate out.
    pub fn takes(self, kind: SolidKind) -> bool {
        match self {
            Mover::Mob { .. } => matches!(kind, SolidKind::Shulker | SolidKind::HappyGhast),
            Mover::Player { .. } | Mover::Passive => matches!(
                kind,
                SolidKind::Shulker | SolidKind::HappyGhast | SolidKind::Boat
            ),
            Mover::Cart { .. } | Mover::Boat { .. } => true,
        }
    }

    /// Whether `c` is solid to this mover. `root` gives the bottom vehicle of `c` and is asked
    /// only when the rule needs it.
    pub fn solid(self, c: &Candidate, root: impl FnOnce() -> Option<u32>) -> bool {
        let ghast_adult = c.alive && !c.baby;
        match self {
            Mover::Mob {
                id,
                happy_ghast,
                mount,
            } => {
                c.id != id
                    && match c.kind {
                        SolidKind::Shulker => c.alive,
                        SolidKind::HappyGhast => {
                            ghast_adult && (c.still || (c.carrying && happy_ghast))
                        }
                        _ => false,
                    }
                    && !(mount == Some(c.id)
                        || c.mount == Some(id)
                        || (mount.is_some() && c.mount == mount))
            }
            Mover::Player {
                feet_y,
                riding,
                side,
            } => match c.kind {
                SolidKind::Shulker => c.alive,
                SolidKind::HappyGhast => {
                    Some(c.id) != riding
                        && ghast_adult
                        && (c.still
                            || (side == Side::Client && feet_y >= c.bounds.max_position().y))
                }
                SolidKind::Boat => Some(c.id) != riding,
                _ => false,
            },
            Mover::Passive => match c.kind {
                SolidKind::Shulker => c.alive,
                SolidKind::HappyGhast => ghast_adult && c.still,
                SolidKind::Boat => true,
                _ => false,
            },
            Mover::Cart { id } => match c.kind {
                SolidKind::Cart => c.id != id,
                SolidKind::Boat | SolidKind::Player => true,
                SolidKind::Shulker | SolidKind::HappyGhast | SolidKind::Mob => {
                    !c.removed && c.mount != Some(id) && vehicle_pushable(c)
                }
            },
            Mover::Boat { id, root: own_root } => {
                c.id != id
                    && !c.removed
                    && (matches!(
                        c.kind,
                        SolidKind::Boat | SolidKind::Cart | SolidKind::Player
                    ) || vehicle_pushable(c))
                    && own_root.is_none_or(|own_root| root() != Some(own_root))
            }
        }
    }
}

/// `Mob::pushable`, plus the living shulkers it leaves out (they take no push, but count).
fn vehicle_pushable(c: &Candidate) -> bool {
    c.pushable || c.kind == SolidKind::Shulker && c.alive
}

/// One mover's question, with the area it asks about when there is one.
#[derive(Clone, Copy, Debug)]
pub struct Solids {
    mover: Mover,
    area: Option<Aabb>,
}

impl Solids {
    pub fn new(mover: Mover, area: Option<Aabb>) -> Self {
        Self { mover, area }
    }

    /// [`Mover::takes`].
    pub fn takes(self, kind: SolidKind) -> bool {
        self.mover.takes(kind)
    }

    /// Whether `c` is solid to the mover and meets the area; `root` is [`Mover::solid`]'s.
    pub fn accepts(self, c: &Candidate, root: impl FnOnce() -> Option<u32>) -> bool {
        self.area.is_none_or(|area| c.bounds.intersects(area)) && self.mover.solid(c, root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::geometry::Vec3;

    /// `HappyGhast.canBeCollidedWith(Player)`: an adult alive ghast is solid to a player while it
    /// is held still, and on the client also to one whose feet are at or over its top; never a
    /// baby or a dead one.
    #[test]
    fn a_players_feet_over_a_happy_ghasts_top_or_its_still_timeout_make_it_solid() {
        let ghast = |alive, baby, still| Candidate {
            alive,
            baby,
            still,
            ..Candidate::fixed(
                SolidKind::HappyGhast,
                7,
                Aabb::new(Vec3::new(0.0, 80.0, 0.0), Vec3::new(4.0, 84.0, 4.0)),
            )
        };
        let solid = |side, feet_y, c: Candidate| {
            Mover::Player {
                feet_y,
                riding: None,
                side,
            }
            .solid(&c, || None)
        };
        let client = |feet_y, c| solid(Side::Client, feet_y, c);
        let server = |feet_y, c| solid(Side::Server, feet_y, c);
        assert!(client(84.0, ghast(true, false, false)));
        assert!(client(85.0, ghast(true, false, false)));
        assert!(!client(83.99, ghast(true, false, false)));
        assert!(client(83.99, ghast(true, false, true)));
        assert!(!client(84.0, ghast(true, true, true)));
        assert!(!client(84.0, ghast(false, false, true)));
        // The server has no predictive landing: only the still timeout makes it solid.
        assert!(!server(85.0, ghast(true, false, false)));
        assert!(server(83.99, ghast(true, false, true)));
        assert!(server(85.0, ghast(true, false, true)));
        assert!(!server(85.0, ghast(true, true, true)));
    }
}
