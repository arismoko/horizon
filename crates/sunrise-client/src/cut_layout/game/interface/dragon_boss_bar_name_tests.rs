//! `EnderDragonFight.updateDragon`: the fight's boss bar takes the dragon's
//! custom name (`dragon.getDisplayName()`), and keeps the type's name until
//! it has one.
use crate::game::load_resources;
use sunrise_client::settings::Settings;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::launch_options::Options;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;

fn end_game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    assert_eq!(game.enter_command_level(Dim::End), Some(Dim::Overworld));
    assert!(game.dimension_type().has_ender_dragon_fight, "the End");
    game
}

#[test]
#[ignore = "daily"]
fn the_dragon_boss_bar_takes_the_dragons_custom_name() {
    let mut resources = None;
    let mut game = end_game(&mut resources);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 128.0, 0.5)).unwrap());
    // The chunks `EnderDragonFight.isArenaLoaded` waits for, lit as received
    // chunks are, so the fight's next player scan adopts or makes its dragon.
    {
        let streaming = &mut game.server.streaming;
        let world = &mut game.common.world;
        for z in -2..=1 {
            for x in -2..=1 {
                streaming.insert_remote(world, Chunk::empty(x, z));
            }
        }
    }
    // `updatePlayers` scans every 20 ticks; then `findOrCreateDragon` makes
    // the fight's dragon.
    for _ in 0..20 {
        game.tick_fight();
    }
    let id = game.dragon.as_ref().expect("the fight's dragon").id;
    let (mut hub, near, _far) = sunrise_gameplay::game::server_side::lan_particles::tests::test_hub(
        "DragonBossBarNameTest",
        Vec::new(),
        None,
    );
    hub.guests.update(0, |guest| {
        guest.dimension = "minecraft:the_end".into();
        guest.position = [0.5, 128.0, 0.5];
    });
    game.server.lan = Some(sunrise_gameplay::game::lan::LanHost::for_test(hub));
    let mut overlay = sunrise_client::boss_overlay::BossOverlay::default();
    let drain = |rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
                 overlay: &mut sunrise_client::boss_overlay::BossOverlay| {
        let now = sunrise_client::boss_overlay::now_millis();
        while let Ok(out) = rx.try_recv() {
            let sunrise_sim::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            let Ok(id) = r.varint() else {
                continue;
            };
            if let Ok(sunrise_protocol::packets::play::Clientbound::BossEvent(event)) =
                sunrise_protocol::packets::play::Clientbound::decode(id, r.rest())
            {
                overlay.apply(&event, now);
            }
        }
    };
    game.publish_boss_bars();
    drain(&near, &mut overlay);
    let default = overlay
        .bars(sunrise_client::boss_overlay::now_millis())
        .pop()
        .expect("the dragon's bar");
    assert_eq!(
        default.name.translation_key(),
        Some("entity.minecraft.ender_dragon")
    );
    // The name a summoned dragon carries: `remember_tags` keeps its
    // `CustomName` for `/data get`, and `updateDragon` reads it.
    game.persist.raw.insert(
        id,
        Tag::compound([("CustomName", Tag::String("Bob".into()))]),
    );
    game.publish_boss_bars();
    drain(&near, &mut overlay);
    let named = overlay
        .bars(sunrise_client::boss_overlay::now_millis())
        .pop()
        .expect("the dragon's bar");
    assert_eq!(named.name.plain(), "Bob");
    // A living NoAI dragon's `aiStep` does not call `updateDragon`, so the
    // bar keeps the name it had.
    game.dragon.as_mut().expect("the fight's dragon").no_ai = true;
    game.persist.raw.insert(
        id,
        Tag::compound([("CustomName", Tag::String("Not Yet".into()))]),
    );
    game.publish_boss_bars();
    drain(&near, &mut overlay);
    let still = overlay
        .bars(sunrise_client::boss_overlay::now_millis())
        .pop()
        .expect("the dragon's bar");
    assert_eq!(still.name.plain(), "Bob");
    game.server.lan = None;
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
