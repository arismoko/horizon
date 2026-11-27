//! `LevelHost::block_states`: a part reads the block-state facts inside a level callback.
use mod_api::{
    BlockPart, BlockState, Flow, Hooks, HostBlockStates, RandomTickEvent, Registrar, WeatheringAge,
};
use std::sync::{Arc, Mutex};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::LevelDimension;

type Facts = (Option<WeatheringAge>, Option<BlockState>, Option<bool>);

/// A stone part whose random tick reads its neighbour's facts through the level view.
#[derive(Clone, Default)]
struct Reader(Arc<Mutex<Vec<(Facts, Option<BlockState>)>>>);

#[mod_api::sunrise_part]
impl BlockPart for Reader {
    fn random_tick(
        &self,
        level: &mut dyn mod_api::LevelWriteHost,
        event: &mut RandomTickEvent,
    ) -> Flow {
        let pos = event.pos;
        let neighbour = level.block_state([pos.x + 1, pos.y, pos.z]).unwrap();
        let facts = level.block_states();
        self.0.lock().unwrap().push((
            (
                facts.weathering_age(neighbour),
                facts.next_weathering_stage(neighbour),
                facts.requires_correct_tool(neighbour),
            ),
            level.block_state_id("minecraft:weathered_copper"),
        ));
        Flow::Continue
    }
}

struct ReaderMod(Reader);

#[mod_api::__subscriptions]
impl Hooks for ReaderMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, self.0.clone());
    }
}

#[test]
#[ignore = "daily"]
fn a_random_tick_part_reads_a_neighbours_weathering_facts_through_the_level() {
    let reader = Reader::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "level-facts",
        Box::new(ReaderMod(reader.clone())),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let state = |game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str| {
                game.pack.assets.registry.default_state(name).unwrap()
            };
            let stone = state(game, "minecraft:stone");
            let exposed = state(game, "minecraft:exposed_copper");
            game.world.set(0, 64, 0, stone).unwrap();
            game.world.set(1, 64, 0, exposed).unwrap();
            let mut handled = false;
            game.as_guest(12, |game| {
                game.with_level(|_, level, ticks| {
                    level.native_random_tick(
                        ticks,
                        LevelDimension::Overworld,
                        BlockPos::new(0, 64, 0),
                        stone,
                        &mut handled,
                    );
                });
            })
            .unwrap();

            // The part's answers are the ones a game host gives for the same state.
            let exposed_api = {
                use sunrise_gameplay_common::game::api_values::TryIntoApi;
                exposed.try_into_api().unwrap()
            };
            let host = game.block_states();
            let host_facts = (
                host.weathering_age(exposed_api),
                host.next_weathering_stage(exposed_api),
                host.requires_correct_tool(exposed_api),
            );
            let seen = reader.0.lock().unwrap();
            let [(facts, weathered)] = &seen[..] else {
                panic!("the part ran {} times", seen.len());
            };
            assert_eq!(facts, &host_facts);
            assert_eq!(facts.0, Some(WeatheringAge::Exposed));
            // Independent of the facts service: the block the level names by its id.
            assert_eq!(facts.1, *weathered);
            assert!(weathered.is_some());
        });
    });
}
