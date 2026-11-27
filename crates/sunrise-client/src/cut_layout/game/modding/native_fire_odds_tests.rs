//! `fire_odds` answered by a block Part with the asked block's position and face (#3185): the
//! Part's answer for one face makes the fire beside that face valid, and the other faces keep
//! vanilla's table.
#![cfg(feature = "client")]
#![cfg(test)]

use mod_api::{self, BlockPart, FireOddsEvent, Hooks, LevelSide, Registrar};
use sunrise_core::{geometry::Direction, position::BlockPos};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::remote::dry_level::DryLevelGameExt2 as _;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

/// Glass that ignites from its west face only while the square south of the fire is air.
struct WestFace {
    pub client: bool,
}

#[mod_api::sunrise_part]
impl BlockPart for WestFace {
    fn fire_odds(&self, host: LevelSide<'_>, event: &mut FireOddsEvent) -> Option<(i32, i32)> {
        assert_eq!(matches!(host, LevelSide::Client(_)), self.client);
        if event.face != Direction::West {
            return None;
        }
        let level = match host {
            LevelSide::Client(level) | LevelSide::Server(level) => level,
        };
        let pos = event.pos;
        let neighbor = level.block_state([pos.x - 1, pos.y, pos.z + 1]);
        let air = level.block_state_id("minecraft:air").expect("air state");
        (neighbor == Some(air)).then_some((5, 0))
    }
}

struct FireOddsMod;

#[mod_api::__subscriptions]
impl Hooks for FireOddsMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let glass = r.block_key("minecraft:glass");
        r.block_part(glass, WestFace { client: false });
        let tinted_glass = r.block_key("minecraft:tinted_glass");
        r.block_part(tinted_glass, WestFace { client: true });
    }
}

#[test]
#[ignore = "daily"]
fn a_block_part_answers_fire_odds_for_one_face() {
    use std::sync::Arc;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;

    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("fire_odds", Box::new(FireOddsMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let fire = registry.default_state("minecraft:fire").unwrap();
            game.world.set(1, 65, 0, glass).unwrap();
            // Fire in the air at `x`, beside the glass, with air below it: it
            // stands only beside a flammable face.
            let stands = |game: &mut sunrise_gameplay::game::ServerGame<'_>, x: i32| {
                game.with_level(|b, level, ticks| {
                    b.can_survive(level, ticks, BlockPos::new(x, 65, 0), fire)
                })
            };
            assert!(stands(game, 0), "the Part makes the west face flammable");
            assert!(!stands(game, 2), "the east face keeps vanilla's never");

            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let level = game.level(game.dim.id()).unwrap();
            let fire = fire.try_into_api().unwrap();
            assert_eq!(
                level.can_survive(game, [0, 65, 0], fire),
                Ok(true),
                "handle survival dispatches the face-sensitive fire-odds Part"
            );
        });
        with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "FireOddsPrediction").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.set_local_mode(sunrise_command::args::GameMode::Survival);

            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:tinted_glass").unwrap();
            let fire = registry.default_state("minecraft:fire").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let air = game.res.facts().air;
            // Putting stone beside the fire forces a dry neighbour shape update.
            // The glass is the only possible support. The west face keeps fire when
            // the placement is north of it, but loses it when stone goes to its south.
            for (x, glass_x, z, stands) in [(2, 3, 1, true), (6, 5, 3, false), (10, 11, 3, false)] {
                game.world.set(glass_x, 65, 2, glass).unwrap();
                game.world.set(x, 65, 2, fire).unwrap();
                game.world.set(x, 64, z, stone).unwrap();
                let hit = sunrise_gameplay_common::game::Hit {
                    pos: [x, 64, z],
                    face: [0, 1, 0],
                    point: [f64::from(x) + 0.5, 65.0, f64::from(z) + 0.5],
                    inside: false,
                };
                let placed = game
                    .remote_can_place(&hit, &ItemStack::new("minecraft:stone", 1))
                    .expect("stone placement prediction");
                assert_eq!(placed.pos, BlockPos::new(x, 65, z));
                let pos = BlockPos::new(x, 65, 2);
                let predicted = placed
                    .changes
                    .iter()
                    .find(|(at, _)| *at == pos)
                    .map_or(fire, |(_, state)| *state);
                if stands {
                    assert_eq!(registry.state(predicted).unwrap().name, "minecraft:fire");
                } else {
                    assert_eq!(
                        predicted, air,
                        "non-flammable face loses the fire at {pos:?}"
                    );
                }
                assert_eq!(game.world.block(x, 65, 2), fire, "fire stays live");
                assert_eq!(game.world.block(x, 65, z), air, "placement stays dry");
            }

            // Stage block updates in the same adapter the fire-odds Part reads.
            game.client.remote.as_mut().unwrap().logged_in = true;
            let campfire = [4, 65, 8];
            let at = BlockPos::new(campfire[0], campfire[1], campfire[2]);
            let campfire_state = registry.default_state("minecraft:campfire").unwrap();
            let hay = registry.default_state("minecraft:hay_block").unwrap();
            game.world.set(at.x, at.y, at.z, campfire_state).unwrap();
            let key = (at.x, at.y, at.z);
            game.world.block_entities.insert(
                key,
                sunrise_level::containers::BlockEntity::Campfire(Box::default()),
            );
            let client_level = game.native_client_level().expect("connected client level");
            let entity = game
                .block_entity(client_level, campfire)
                .expect("campfire entity");
            let original_data = game.with_level(|_, live, _| {
                mod_api::LevelHost::block_entity_data(live, entity).expect("campfire data")
            });
            let foreign = mod_api::__private::game::block_entity_ref(
                mod_api::LevelRef::new(client_level.epoch(), mod_api::DimensionKey::NETHER),
                campfire,
                game.world.block_entities.generation(&key).unwrap(),
            );
            game.check_dry_fire_odds_view(
                |b, dry, ticks| {
                    assert!(b.set_block(dry, ticks, at.offset(0, -1, 0), hay, 3));
                },
                |host| {
                    let staged = host.block_state(campfire).unwrap();
                    assert!(
                        host.state_string(staged)
                            .unwrap()
                            .contains("signal_fire=true"),
                        "the hay bale changed the campfire state"
                    );
                    assert_eq!(host.block_entity(campfire), Some(entity));
                    assert_eq!(host.block_entity_data(entity), Some(original_data));
                },
            );
            game.check_dry_fire_odds_view(
                |b, dry, ticks| {
                    assert!(b.set_block(dry, ticks, at, stone, 3));
                },
                |host| {
                    assert!(host.block_entity(campfire).is_none());
                    assert!(host.block_entity_data(entity).is_none());
                    assert!(matches!(
                        host.block_entity_part_snapshot(entity, "fire_odds", "test"),
                        Err(mod_api::PartStateError::Stale)
                    ));
                    assert!(matches!(
                        host.block_entity_part_snapshot(foreign, "fire_odds", "test"),
                        Err(mod_api::PartStateError::OtherLevel)
                    ));
                },
            );
            assert_eq!(game.world.block(at.x, at.y, at.z), campfire_state);
            assert_eq!(game.block_entity(client_level, campfire), Some(entity));

            sunrise_gameplay_common::game::growth_features::install_live_features(
                &game.res,
                &game.pack,
                &game.options,
                &Default::default(),
            );
            game.world.weather.raining = true;
            game.world.weather.settle_levels();
            let rain = [12, 65, 10];
            let plains = game.res.biomes().id("minecraft:plains");
            let [qx, qy, qz] = game
                .client
                .streaming
                .biome_zoom
                .quart_at_block(BlockPos::new(rain[0], rain[1], rain[2]));
            let column = (qx >> 2, qz >> 2);
            let local_qy = qy - (game.world.bottom_y() >> 2);
            Arc::make_mut(game.world.chunks.get_mut(&column).unwrap())
                .set_biome_cell(qx, local_qy, qz, plains);
            let raining_live = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                game.with_level(|_, live, _| mod_api::LevelHost::is_raining_at(live, rain))
            };
            assert!(raining_live(game), "the live sky is open to rain");
            game.check_dry_fire_odds_view(
                |_, _, _| {},
                |host| {
                    assert!(
                        host.is_raining_at(rain),
                        "an open dry column still sees rain"
                    )
                },
            );
            let roof = BlockPos::new(rain[0], rain[1] + 1, rain[2]);
            game.check_dry_fire_odds_view(
                |b, dry, ticks| assert!(b.set_block(dry, ticks, roof, stone, 3)),
                |host| assert!(!host.is_raining_at(rain), "the staged roof blocks rain"),
            );
            assert!(
                raining_live(game),
                "the roof was not placed in the live level"
            );
        });
    });
}

/// `LevelRef::fire_odds` reads the odds fire itself uses: the Part's answer for the face it
/// answers, vanilla's table for the other faces and for blocks without a Part, and a stale level
/// is refused.
#[test]
#[ignore = "daily"]
fn the_fire_odds_read_takes_the_parts_answer_then_vanillas_table() {
    use std::sync::Arc;
    use sunrise_gameplay_common::game::api_values::{TryIntoApi, install_parts};
    use sunrise_level::chunk::Chunk;

    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("fire_odds", Box::new(FireOddsMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        install_parts(&mods).unwrap();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| -> mod_api::BlockState {
                registry
                    .default_state(name)
                    .unwrap()
                    .try_into_api()
                    .unwrap()
            };
            let level = game.level(game.dim.id()).unwrap();
            let glass = state("minecraft:glass");
            let odds = |game: &sunrise_gameplay::game::ServerGame<'_>, state, face| {
                level.fire_odds(game, [1, 65, 0], state, face)
            };
            assert_eq!(odds(game, glass, Direction::West), Ok((5, 0)));
            assert_eq!(odds(game, glass, Direction::East), Ok((0, 0)));
            // FireBlock.bootStrap: planks 5 and 20, wool 30 and 60.
            assert_eq!(
                odds(game, state("minecraft:oak_planks"), Direction::Up),
                Ok((5, 20))
            );
            assert_eq!(
                odds(game, state("minecraft:white_wool"), Direction::Up),
                Ok((30, 60))
            );
            let stale = mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert_eq!(
                stale.fire_odds(game, [1, 65, 0], glass, Direction::West),
                Err(mod_api::LevelReadError::Stale)
            );
        });
    });
}

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
