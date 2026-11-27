//! The game's inputs to `EntityGetter.getEntityCollisions`: the entity lists, the spatial
//! queries and the stand's root-vehicle grid, offered to the one rule in
//! `sunrise_client::entity_solids` (which a mob's own move reads too).
use super::{boats_common::Boat, commands_common::Target, minecarts::Minecart};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_sim::entity_solids::{Candidate, Mover as Rule, Side, SolidKind, Solids};

/// The mover asking, as the game's callers name it.
#[derive(Clone, Copy)]
pub enum Mover<'a> {
    /// The window's player.
    Player,
    /// Any player that is not riding, by the height of its feet: a guest the server holds.
    PlayerAt { feet_y: f64 },
    /// A mover that is neither a player nor a vehicle: an item, a stand, an orb, TNT, a
    /// falling block, a body a piston pushes.
    Passive,
    /// A minecart, within two blocks of itself.
    Cart(&'a Minecart),
    /// Boat `id` within `area`.
    Boat { id: u32, area: Aabb },
}

/// A boat as a solid, in the box a client's copy takes from its `entity_size` answer.
pub fn boat(copy_box: impl FnOnce(u32, Vec3, Aabb) -> Aabb, b: &Boat) -> Candidate {
    let bounds = copy_box(b.id, b.position, b.bounds());
    Candidate::fixed(SolidKind::Boat, b.id, bounds)
}

/// [`boat`] for a minecart.
pub fn cart(copy_box: impl FnOnce(u32, Vec3, Aabb) -> Aabb, c: &Minecart) -> Candidate {
    let bounds = copy_box(c.id, c.position, c.bounds());
    Candidate::fixed(SolidKind::Cart, c.id, bounds)
}

/// A player the rule may offer; the rule never reads its id.
pub fn player(bounds: Aabb) -> Candidate {
    Candidate::fixed(SolidKind::Player, 0, bounds)
}

/// What a moving cart asks about: two blocks around itself.
pub fn cart_reach(c: &Minecart) -> Aabb {
    c.bounds().inflate(Vec3::new(2.0, 2.0, 2.0))
}

pub trait CollisionSourceGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The boxes of the solid entities for `mover`, in the order its inputs are read.
    fn entity_collisions(&self, mover: Mover<'_>) -> Vec<Aabb> {
        let mut boxes = Vec::new();
        self.each_solid(mover, |c, _| boxes.push(c.bounds));
        boxes
    }

    /// Asks the rule about every entity `mover` may meet, and hands the solid ones to `each`
    /// with the index of its mob.
    fn each_solid(&self, mover: Mover<'_>, mut each: impl FnMut(&Candidate, Option<usize>)) {
        let (rule, area) = match mover {
            Mover::Player => {
                let Some(p) = &self.player else {
                    return;
                };
                (
                    Rule::Player {
                        feet_y: p.position.y,
                        riding: self.riding,
                        side: if self.side_has_remote() {
                            Side::Client
                        } else {
                            Side::Server
                        },
                    },
                    None,
                )
            }
            Mover::PlayerAt { feet_y } => (
                Rule::Player {
                    feet_y,
                    riding: None,
                    side: Side::Server,
                },
                None,
            ),
            Mover::Passive => (Rule::Passive, None),
            Mover::Cart(c) => (Rule::Cart { id: c.id }, Some(cart_reach(c))),
            Mover::Boat { id, area } => (
                Rule::Boat {
                    id,
                    root: self.side_boat_collision_root(id, Target::Boat(id)),
                },
                Some(area),
            ),
        };
        let solids = Solids::new(rule, area);
        match mover {
            Mover::Player | Mover::PlayerAt { .. } | Mover::Passive => {
                for (j, m) in self.mobs.iter().enumerate() {
                    if solids.takes(m.solid_kind()) {
                        let c = m.solid_candidate();
                        if solids.accepts(&c, || None) {
                            each(&c, Some(j));
                        }
                    }
                }
                for b in self
                    .boats
                    .iter()
                    .map(|b| boat(|id, p, bounds| self.client_copy_box(id, p, bounds), b))
                {
                    if solids.accepts(&b, || None) {
                        each(&b, None);
                    }
                }
            }
            Mover::Cart(_) | Mover::Boat { .. } => {
                self.side_vehicle_solids(mover, &solids, &mut each);
            }
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CollisionSourceGameExt1<'__sg> for G {}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
