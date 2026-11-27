//! `ignited_by_lava` answered by a block Part with the asked block's face (#3185): the Part's
//! answer replaces the state's `IGNITED_BY_LAVA` flag either way.
use mod_api::{self, BlockPart, Hooks, IgnitedByLavaEvent, LevelHost, Registrar};
use sunrise_core::{geometry::Direction, position::BlockPos};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_world::{level::LevelAccess, registry::BlockStateId};

/// Stone that lava lights from its top only.
struct TopIgnites;

#[mod_api::sunrise_part]
impl BlockPart for TopIgnites {
    fn ignited_by_lava(&self, _: &dyn LevelHost, event: &mut IgnitedByLavaEvent) -> Option<bool> {
        Some(event.face == Direction::Up)
    }
}

struct LavaMod;

#[mod_api::__subscriptions]
impl Hooks for LavaMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, TopIgnites);
    }
}

#[test]
#[ignore = "daily"]
fn a_block_part_decides_whether_lava_lights_it() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("lava", Box::new(LavaMod))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let planks = registry.default_state("minecraft:oak_planks").unwrap();
            let ignites = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                           state: BlockStateId,
                           face: Direction| {
                game.with_level(|_, level, _| {
                    let mut ignites = None;
                    level.before_ignited_by_lava(
                        state,
                        BlockPos::new(0, 64, 0),
                        face,
                        &mut ignites,
                    );
                    ignites
                })
            };
            assert_eq!(ignites(game, stone, Direction::Up), Some(true));
            assert_eq!(ignites(game, stone, Direction::North), Some(false));
            assert_eq!(
                ignites(game, planks, Direction::Up),
                None,
                "no Part: the flag answers"
            );
            // A client's copy of the level asks no Part.
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            assert_eq!(ignites(game, stone, Direction::Up), None);
        })
    });
}
