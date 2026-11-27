#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_game_role_client;
use mod_api::{BlockPart, Hooks, LevelSide, Registrar, SmokeSourceEvent};
use std::sync::Arc;
use sunrise_gameplay_common::game::remote::ItemStack;

struct Bale;
#[mod_api::sunrise_part]
impl BlockPart for Bale {
    fn smoke_source(&self, host: LevelSide<'_>, _: &mut SmokeSourceEvent) -> Option<bool> {
        assert!(matches!(host, LevelSide::Client(_)));
        Some(true)
    }
}

struct SmokeMod;
#[mod_api::__subscriptions]
impl Hooks for SmokeMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let bale = r.block_key("smoke_test:bale");
        r.block_part(bale, Bale);
    }
}

#[test]
#[ignore = "daily"]
fn client_placement_predicts_signal_fire_from_a_block_part() {
    use sunrise_mod_package::content::{Block, Content};
    use sunrise_sim::mod_content::{ContentMod, build};

    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = tempfile::tempdir().unwrap();
    let mut content = Content::declared("smoke_test");
    content
        .declare(
            &[Block {
                id: "smoke_test:bale".into(),
                template: "minecraft:stone".into(),
                item: true,
                ..Default::default()
            }],
            &[],
        )
        .unwrap();
    let inventory = sunrise_mod_package::enabled::EnabledMods::new([(
        "smoke_test".to_owned(),
        mod_api::Side::Both,
    )]);
    build(
        &base,
        std::path::Path::new(""),
        &[ContentMod {
            id: "smoke_test".into(),
            content,
            jars: Vec::new(),
        }],
        root.path(),
        &inventory,
    )
    .unwrap();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("smoke_test", Box::new(SmokeMod))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game_role_client(
            root.path().to_path_buf(),
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |game| {
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "SmokePrediction").unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.set_local_mode(sunrise_command::args::GameMode::Survival);
                for (i, (block, expected)) in [
                    ("smoke_test:bale", true),
                    ("minecraft:hay_block", true),
                    ("minecraft:dirt", false),
                ]
                .into_iter()
                .enumerate()
                {
                    let x = i as i32 * 2;
                    let below = game.pack.assets.registry.default_state(block).unwrap();
                    game.world.set(x, 64, 0, below).unwrap();
                    let hit = sunrise_gameplay_common::game::Hit {
                        pos: [x, 64, 0],
                        face: [0, 1, 0],
                        point: [f64::from(x) + 0.5, 65.0, 0.5],
                        inside: false,
                    };
                    let preview = game
                        .remote_can_place(&hit, &ItemStack::new("minecraft:campfire", 1))
                        .expect("campfire prediction");
                    let pos = sunrise_core::position::BlockPos::new(x, 65, 0);
                    assert_eq!(preview.pos, pos);
                    let predicted = preview.changes.iter().find(|(at, _)| *at == pos).unwrap().1;
                    let state = game.pack.assets.registry.state(predicted).unwrap();
                    assert_eq!(
                        state.properties.get("signal_fire").map(String::as_str),
                        Some(if expected { "true" } else { "false" }),
                        "{block}"
                    );
                    assert_eq!(
                        game.world.block(x, 65, 0),
                        game.res.facts().air,
                        "prediction stays dry"
                    );

                    // Place the source under an existing cosy campfire instead.
                    let campfire = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:campfire")
                        .unwrap();
                    let ground = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(x, 63, 0, ground).unwrap();
                    game.common
                        .world
                        .set(x, 64, 0, game.common.res.facts().air)
                        .unwrap();
                    game.world.set(x, 65, 0, campfire).unwrap();
                    let hit = sunrise_gameplay_common::game::Hit {
                        pos: [x, 63, 0],
                        face: [0, 1, 0],
                        point: [f64::from(x) + 0.5, 64.0, 0.5],
                        inside: false,
                    };
                    let preview = game
                        .remote_can_place(&hit, &ItemStack::new(block, 1))
                        .expect("placement under campfire");
                    assert_eq!(preview.pos, sunrise_core::position::BlockPos::new(x, 64, 0));
                    let predicted = preview
                        .changes
                        .iter()
                        .find(|(at, _)| *at == pos)
                        .map_or(campfire, |(_, state)| *state);
                    let state = game.pack.assets.registry.state(predicted).unwrap();
                    assert_eq!(
                        state.properties.get("signal_fire").map(String::as_str),
                        Some(if expected { "true" } else { "false" }),
                        "neighbour shape update over {block}"
                    );
                    assert_eq!(
                        game.world.block(x, 65, 0),
                        campfire,
                        "neighbour prediction stays dry"
                    );
                }
            },
        );
    });
}
