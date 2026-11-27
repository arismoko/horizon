//! Target dispatch: the level, position and type a command's target names,
//! by family. Moved out of `mod.rs` unchanged (#2791's discipline).
use crate::game::server_side::commands::*;

/// The network id of a target that is no player: its entity id.
pub(in crate::game) fn non_player_entity_id(t: Target) -> Option<i32> {
    let (Target::Mob(id)
    | Target::Item(id)
    | Target::Orb(id)
    | Target::Projectile(id)
    | Target::Stand(id)
    | Target::Frame(id)
    | Target::Painting(id)
    | Target::Boat(id)
    | Target::Cart(id)
    | Target::Other(id)
    | Target::Fangs(id)
    | Target::Crystal(id)
    | Target::Dragon(id)
    | Target::DragonPart(id)
    | Target::Tnt(id)
    | Target::Cushion(id)
    | Target::Falling(id)
    | Target::Rocket(id)
    | Target::Trident(id)
    | Target::Spawner(id)
    | Target::Cloud(id)
    | Target::Bolt(id)
    | Target::Hook(id)
    | Target::General(id)
    | Target::Knot(id)) = t
    else {
        return None;
    };
    i32::try_from(id).ok()
}
