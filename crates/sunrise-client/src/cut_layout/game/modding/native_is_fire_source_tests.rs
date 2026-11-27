//! `is_fire_source` answered by a block Part (#3185): stone keeps a fire burning, netherrack
//! refuses what the overworld's infiniburn tag gives it, and other blocks leave the tag to answer.
use mod_api::{self, BlockPart, Hooks, IsFireSourceEvent, LevelHost, Registrar};
use sunrise_core::position::BlockPos;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_world::level::LevelAccess;

/// Answers `is_fire_source` for its block.
struct Source(bool);

#[mod_api::sunrise_part]
impl BlockPart for Source {
    fn is_fire_source(&self, _: &dyn LevelHost, _: &mut IsFireSourceEvent) -> Option<bool> {
        Some(self.0)
    }
}

struct FireSourceMod;

#[mod_api::__subscriptions]
impl Hooks for FireSourceMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, Source(true));
        let netherrack = r.block_key("minecraft:netherrack");
        r.block_part(netherrack, Source(false));
    }
}

#[test]
#[ignore = "daily"]
fn a_block_part_decides_whether_it_keeps_a_fire_burning() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "fire_source",
        Box::new(FireSourceMod),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let source = |game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str| {
                let state = registry.default_state(name).unwrap();
                game.with_level(|_, level, _| {
                    let mut source = None;
                    level.before_is_fire_source(state, BlockPos::new(0, 64, 0), &mut source);
                    source
                })
            };
            assert_eq!(source(game, "minecraft:stone"), Some(true));
            assert_eq!(source(game, "minecraft:netherrack"), Some(false));
            assert_eq!(
                source(game, "minecraft:dirt"),
                None,
                "no Part: the tag answers"
            );
            // A client's copy of the level asks no Part.
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            assert_eq!(source(game, "minecraft:stone"), None);
        })
    });
}
