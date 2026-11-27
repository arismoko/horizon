//! `Bee.doHurtTarget`: the stinger, the calm-down and the sting sound follow
//! only a hit that landed. A creative player, a raised shield and an
//! invulnerable mob refuse it, and the bee keeps its stinger.
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::{Mob, Who, ai::do_hurt_target, goals::Cx};

pub fn spawn(game: &mut crate::game::ServerGame<'_>, id: u32, kind: &str, position: Vec3) -> u32 {
    let kind = game.common.res.mob_types().get(kind).unwrap().clone();
    game.common.mobs.push(Mob::new(id, kind, position, 0.0));
    id
}

/// One `Bee.doHurtTarget` from the bee, then the game's event handling.
pub fn sting(game: &mut crate::game::ServerGame<'_>, bee_id: u32, who: Who) {
    let mut mobs = game.common.mobs.lift();
    let i = mobs.iter().position(|m| m.id == bee_id).unwrap();
    let mut test_level = sunrise_sim::health::TestLevel::new(0);
    let mut events = Vec::new();
    {
        let w = game.mob_world_base(&[]);
        let mut cx = Cx {
            w: &w,
            level: &mut test_level.level(),
            events: &mut events,
        };
        do_hurt_target(&mut mobs[i], who, &mut cx);
    }
    game.common
        .mobs
        .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
    game.handle_mob_events(events).unwrap();
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
