use crate::game::native_entity_refs::tests::{with_client_game, with_game};
use mod_api::{
    BlockPart, BlockPos, ClientHost, Flow, Hooks, ModEntry, PlayBreakSoundEvent, Registrar,
};
use std::sync::{Arc, Mutex};
use sunrise_client_gameplay::game::{
    ClientGame, client_side::native_client_session::tests::logged_client,
};
use sunrise_client_render::particles::ParticleLevel;
use sunrise_gameplay_common::game::side_ops::events::EventsSideGameExt as _;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;
use sunrise_world::level::LevelEvent;

type Asked = Arc<Mutex<Vec<BlockPos>>>;
struct Quiet(Asked);
#[mod_api::sunrise_part]
impl BlockPart for Quiet {
    fn play_break_sound(&self, _: &mut dyn ClientHost, event: &mut PlayBreakSoundEvent) -> Flow {
        self.0.lock().unwrap().push(event.pos);
        Flow::Cancel
    }
}
struct Register(Asked);
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("minecraft:glass");
        r.block_part(block, Quiet(self.0.clone()));
    }
}

/// The particles spawned since the last clear, once the particle tick admits them.
fn admitted(game: &mut ClientGame<'_>) -> usize {
    game.client.particles.0.tick(&ParticleLevel {
        world: &game.common.world,
        facts: game.common.res.facts(),
        collision: game.common.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    });
    game.client.particles.0.len()
}

/// Level event 2001 for glass, whose part cancels, plays no break sound but still spawns its
/// debris; stone keeps vanilla's sound; air asks nothing.
#[test]
#[ignore = "daily"]
fn a_cancelling_part_silences_level_event_2001_and_keeps_the_debris() {
    let asked = Asked::default();
    let mods = vec![ModEntry::new("quiet", Box::new(Register(asked.clone())))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(
                PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(4.5, 63.0, 4.5),
                )
                .unwrap(),
            );
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = &game.pack.assets.registry;
            let [stone, glass, air] = ["minecraft:stone", "minecraft:glass", "minecraft:air"]
                .map(|name| registry.default_state(name).unwrap());
            for state in [stone, glass, air] {
                game.client.audio.0.capture = Some(Vec::new());
                asked.lock().unwrap().clear();
                game.client.particles.0.clear();
                game.level_events.push(LevelEvent::LevelEvent {
                    source: None,
                    id: 2001,
                    pos: sunrise_core::position::BlockPos::new(5, 63, 4),
                    data: state.0 as i32,
                });
                game.side_process_level_events().unwrap();
                let played = game.client.audio.0.capture.take().unwrap();
                if state == stone {
                    assert_eq!(played.len(), 1);
                    assert_eq!(played[0].event, "minecraft:block.stone.break");
                    assert_eq!(played[0].position, [5.5, 63.5, 4.5]);
                    assert_eq!((played[0].volume, played[0].pitch), (1.0, 0.8));
                } else {
                    assert!(played.is_empty());
                }
                let expected: &[BlockPos] = if state == glass {
                    &[BlockPos::new(5, 63, 4)]
                } else {
                    &[]
                };
                assert_eq!(*asked.lock().unwrap(), expected);
                assert_eq!(admitted(game) > 0, state != air);
            }
        })
    });
}

/// The hook is the client's: the server thread, which handles level event 2001 too, never asks
/// a part, so a single-player break asks it once.
#[test]
#[ignore = "daily"]
fn the_server_thread_never_asks_a_part() {
    let asked = Asked::default();
    let mods = vec![ModEntry::new("quiet", Box::new(Register(asked.clone())))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let glass = game
                .pack
                .assets
                .registry
                .default_state("minecraft:glass")
                .unwrap();
            game.level_events.push(LevelEvent::LevelEvent {
                source: None,
                id: 2001,
                pos: sunrise_core::position::BlockPos::new(5, 63, 4),
                data: glass.0 as i32,
            });
            game.side_process_level_events().unwrap();
            assert!(asked.lock().unwrap().is_empty());
        })
    });
}
