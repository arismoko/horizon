use super::*;
use sunrise_core::random::LegacyRandom;

pub const SEED: i64 = 2;
pub const AT: [f64; 3] = [10.0, 64.0, 20.0];

pub fn breed_pair(game: &mut crate::game::ServerGame<'_>, kind: &str) -> (u32, u32, Option<u32>) {
    breed_pair_credited(game, kind, false)
}

/// [`breed_pair`] with the host player credited for feeding the parents into love (what the
/// goal records in `love_cause`), when `credited`.
pub fn breed_pair_credited(
    game: &mut crate::game::ServerGame<'_>,
    kind: &str,
    credited: bool,
) -> (u32, u32, Option<u32>) {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let base = game.server.next_entity;
    game.server.next_entity += 2;
    game.mobs.push(Mob::new(
        base + 1,
        kind.clone(),
        Vec3::new(0.0, 64.0, 0.0),
        0.0,
    ));
    game.mobs
        .push(Mob::new(base + 2, kind, Vec3::new(1.0, 64.0, 0.0), 0.0));
    game.native_joins_skip();
    if credited {
        game.server.love_cause.insert(base + 1, None);
    }
    let parent = game.mobs.iter_mut().find(|m| m.id == base + 1).unwrap();
    parent.random = LegacyRandom::new(0);
    let live_baby = !matches!(
        parent.species(),
        Species::Turtle | Species::Frog | Species::Sniffer
    );
    if mod_api::__private::listened(mod_api::HookId::BabyEntitySpawn) && live_baby {
        // The listener sees the initiating parent before its deferred cooldown.
        parent.extra.age = 0;
        parent.extra.in_love = 600;
    } else {
        parent.extra.age = 6000;
    }
    let partner = game.mobs.iter_mut().find(|m| m.id == base + 2).unwrap();
    partner.extra.in_love = 600;
    let child = game.breed(
        base + 1,
        base + 2,
        None,
        None,
        None,
        sunrise_sim::mobs::BreedExperience::Deferred,
        true,
    );
    (base + 1, base + 2, child)
}

/// A dying cube mob of `kind` and `size` whose own random is `new Random(seed)`.
pub fn cube(game: &mut crate::game::ServerGame<'_>, kind: &str, size: i32, seed: i64) -> Mob {
    game.server.next_entity += 1;
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut mob = Mob::new(
        game.server.next_entity,
        kind,
        Vec3::new(AT[0], AT[1], AT[2]),
        0.0,
    );
    if mob.species() == Species::SulfurCube {
        sunrise_sim::mobs::sulfur_cube::set_size(&mut mob, size, true);
    } else {
        sunrise_sim::mobs::cube::set_size(&mut mob, size);
    }
    mob.random = LegacyRandom::new(seed);
    mob
}

/// Two cows bred by the real route; the parent's id ends in `mode`.
pub fn breed(game: &mut crate::game::ServerGame<'_>, mode: u32) -> (u32, u32, Option<u32>) {
    game.common.mobs.clear();
    game.server.next_entity = next_parent_id(game, mode) - 1;
    breed_pair(game, "minecraft:cow")
}

/// Select the fixture mode without reusing an ID issued to an earlier mob or breeding orb.
pub(in crate::game::server_side) fn next_parent_id(
    game: &crate::game::ServerGame<'_>,
    mode: u32,
) -> u32 {
    (game.server.next_entity + 1).next_multiple_of(10) + mode
}

/// A size-4 slime whose entity id ends in `mode`, split by the real route.
pub fn split(game: &mut crate::game::ServerGame<'_>, mode: u32) -> Vec<Mob> {
    game.common.mobs.clear();
    let mut dying = cube(game, "minecraft:slime", 4, SEED);
    game.server.next_entity = next_parent_id(game, mode);
    dying.id = game.server.next_entity;
    game.split_cube_children(&mut dying);
    game.common.mobs.to_vec()
}
