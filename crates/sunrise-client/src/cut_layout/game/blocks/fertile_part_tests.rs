//! A farmland part answers `is_fertile` and `can_be_hydrated` (`IBlockExtension.isFertile`,
//! `canBeHydrated`) through the game's level, on an authoritative server only.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{BlockPart, CanBeHydratedEvent, Hooks, IsFertileEvent, LevelHost, Registrar};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::{BlockPos, GameGameExt1 as _, SideGame};
use sunrise_world::level::LevelAccess;

#[derive(Clone)]
struct Farmland;
#[mod_api::sunrise_part]
impl BlockPart for Farmland {
    fn is_fertile(&self, _: &dyn LevelHost, event: &mut IsFertileEvent) {
        event.fertile = !event.fertile;
    }
    fn can_be_hydrated(&self, _: &dyn LevelHost, event: &mut CanBeHydratedEvent) -> Option<bool> {
        Some(mod_api::__private::fluid_id(event.fluid.fluid) == Some("minecraft:lava"))
    }
}
struct Parts;
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let key = r.block_key("minecraft:farmland");
        r.block_part(key, Farmland);
    }
}

#[test]
#[ignore = "daily"]
fn a_farmland_part_answers_fertility_and_hydration() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("farmland", Box::new(Parts))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        // A client's copy of the level: no part is asked.
        with_client_game(|game| {
            let (fertile_asked, farmland_asked, ..) = ask(game);
            assert!(!fertile_asked && !farmland_asked);
        });
        // A server: the farmland's part answers, stone has none.
        with_game(|game| {
            assert_eq!(
                ask(game),
                (true, true, false, true, Some(true), Some(false))
            );
        });
    });
}

fn ask<'g>(game: &mut impl SideGame<'g>) -> (bool, bool, bool, bool, Option<bool>, Option<bool>) {
    let registry = game.common().pack.assets.registry;
    let dry = registry.default_state("minecraft:farmland").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let lava = game
        .common()
        .res
        .behaviours()
        .fluid(registry.default_state("minecraft:lava").unwrap());
    let water = game
        .common()
        .res
        .behaviours()
        .fluid(registry.default_state("minecraft:water").unwrap());
    let (pos, at) = (BlockPos::new(0, 64, 0), BlockPos::new(2, 64, 0));
    game.with_level(|_, level, _| {
        let mut fertile = false;
        level.before_is_fertile(dry, pos, &mut fertile);
        let hydrated = |fluid| {
            let mut hydrates = None;
            level.before_can_be_hydrated(dry, pos, fluid, at, &mut hydrates);
            hydrates
        };
        (
            level.asks_is_fertile(),
            level.asks_can_be_hydrated(dry),
            level.asks_can_be_hydrated(stone),
            fertile,
            hydrated(lava),
            hydrated(water),
        )
    })
}
