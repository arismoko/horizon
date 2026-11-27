#![cfg(test)]
//! `EntityRef::previous_position`, `previous_rotation` and `position_at` on the server:
//! the values `Entity.setOldPosAndRot` kept when the entity's tick began.

use super::native_entity_refs::tests::with_game;
use crate::game::ServerGame;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::Mob;

/// A dedicated server's game with the column at (0, 0) loaded and forced.
fn server_level(game: &mut ServerGame<'_>) {
    game.options.dedicated = Some(Vec::new());
    game.native_level_opened(Dim::Overworld);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.server.streaming.forced.insert((0, 0));
    game.native_joins_skip();
}

/// Summons `kind` at `at` with the NBT `tags` on a server, and returns its id.
fn summon(game: &mut ServerGame<'_>, kind: &str, at: [f64; 3], tags: Vec<(&str, Tag)>) -> u32 {
    let nbt: Compound = tags.into_iter().map(|(k, v)| (k.into(), v)).collect();
    let level = game.level("minecraft:overworld").expect("the overworld");
    game.level_summon(level, kind, at, nbt)
        .expect("a level write")
        .expect("summoned")
        .id()
}

fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(values.map(|v| Tag::Double(v.to_bits())).to_vec())
}

fn floats(values: [f32; 2]) -> Tag {
    Tag::List(values.map(|v| Tag::Float(v.to_bits())).to_vec())
}

/// The mob `id` of the game's list.
fn mob<'a>(game: &'a mut ServerGame<'_>, id: u32) -> &'a mut Mob {
    game.mobs.iter_mut().find(|m| m.id == id).expect("the mob")
}

/// A villager walking on a server: after each tick's `commonTick`, its previous position and
/// rotation are where the tick began, one tick behind what its move left.
#[test]
#[ignore = "daily"]
fn a_walking_villagers_previous_tick_on_the_server() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_game(|game| {
            server_level(game);
            let id = summon(
                game,
                "minecraft:villager",
                [1.5, 70.0, 1.5],
                vec![("NoAI", Tag::Byte(1)), ("Rotation", floats([30.0, 10.0]))],
            );
            let entity = game.entity(id).expect("its handle");
            game.tick_mobs().unwrap();
            // Its walk this tick, as its travel leaves it.
            let m = mob(game, id);
            m.position = Vec3::new(1.75, 70.0, 1.25);
            m.yaw = 40.0;
            m.head_pitch = 5.0;
            let host = &*game as &dyn mod_api::Host;
            assert_eq!(entity.previous_position(host), Some([1.5, 70.0, 1.5]));
            assert_eq!(entity.previous_rotation(host), Some([30.0, 10.0]));
            assert_eq!(entity.position(host), Some([1.75, 70.0, 1.25]));
            assert_eq!(entity.rotation(host), Some([40.0, 5.0]));
            game.tick_mobs().unwrap();
            let host = &*game as &dyn mod_api::Host;
            assert_eq!(entity.previous_position(host), Some([1.75, 70.0, 1.25]));
            assert_eq!(entity.previous_rotation(host), Some([40.0, 5.0]));
            let stale = mod_api::EntityRef::new(entity.epoch(), 9_999);
            assert_eq!(stale.previous_position(host), None);
            assert_eq!(stale.previous_rotation(host), None);
        });
    });
}

/// An arrow on a server: its previous position is where it was before its last move
/// (`AbstractArrow.tick` moves it by its whole motion, 1 block east, before drag and gravity).
#[test]
#[ignore = "daily"]
fn an_arrows_previous_position_is_before_its_last_move() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_game(|game| {
            server_level(game);
            let id = summon(
                game,
                "minecraft:arrow",
                [1.5, 70.0, 1.5],
                vec![("Motion", doubles([1.0, 0.0, 0.0]))],
            );
            let entity = game.entity(id).expect("its handle");
            game.tick_mobs().unwrap();
            let host = &*game as &dyn mod_api::Host;
            assert_eq!(entity.previous_position(host), Some([1.5, 70.0, 1.5]));
            assert_eq!(entity.position(host), Some([2.5, 70.0, 1.5]));
        });
    });
}

/// `Entity.getPosition(float)`: `Mth.lerp(partial, xo, x)` on each axis.
#[test]
#[ignore = "daily"]
fn position_at_eases_from_the_previous_position() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_game(|game| {
            server_level(game);
            let id = summon(
                game,
                "minecraft:villager",
                [1.5, 70.0, 1.5],
                vec![("NoAI", Tag::Byte(1))],
            );
            let entity = game.entity(id).expect("its handle");
            game.tick_mobs().unwrap();
            mob(game, id).position = Vec3::new(2.25, 70.5, 1.0);
            let host = &*game as &dyn mod_api::Host;
            // 1.5 + 0.5 * (2.25 - 1.5), 70 + 0.5 * 0.5, 1.5 + 0.5 * (1.0 - 1.5).
            assert_eq!(entity.position_at(host, 0.5), Some([1.875, 70.25, 1.25]));
            assert_eq!(
                entity.position_at(host, 0.25),
                Some([1.6875, 70.125, 1.375])
            );
            assert_eq!(entity.position_at(host, 0.0), Some([1.5, 70.0, 1.5]));
            assert_eq!(entity.position_at(host, 1.0), Some([2.25, 70.5, 1.0]));
            let stale = mod_api::EntityRef::new(entity.epoch(), 9_999);
            assert_eq!(stale.position_at(host, 0.5), None);
        });
    });
}
