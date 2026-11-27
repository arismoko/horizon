//! `RamTarget.tick`: the push a ram gives, and the shield that softens it
//! (`applyItemBlocking`'s `blockingFactor`).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_gameplay_common::game::{
    ItemStack, Options, PlayerMotion, Progress, Resources, Vec3, execution_role,
};
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::{Damage, MobEvent, Who};

fn game<'a>(resources: &'a mut Option<Arc<Resources>>) -> sunrise_gameplay::game::ServerGame<'a> {
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    // Facing the goat, which stands to the south.
    game.yaw = 0.0;
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    assert!(game.run_command("summon minecraft:goat 0.5 64 1.5 {NoAI:1b,PersistenceRequired:1b}"));
    game
}

/// `RamTarget.tick` on a goat with `speedFactor` 3.0 and force 2.5: the hit's
/// 2 damage, then the push away from the goat.
#[test]
#[ignore = "daily"]
fn a_shield_halves_the_push_of_a_ram() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let goat = game.mobs[0].id;
    let from = [0.5, 64.0, 1.5];
    let ram = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
        game.handle_mob_events(vec![MobEvent::Knockback {
            target: Who::Player,
            power: 7.5,
            x: 0.0,
            z: 1.0,
            source: Damage::mob(goat, from),
            apply_blocking: Some(2.0),
        }])
        .unwrap();
        game.player.as_ref().unwrap().velocity
    };
    // An open victim takes the whole push.
    assert_eq!(ram(&mut game).z, -7.5, "an unblocked ram's push");
    // A raised shield blocks the hit (`blockingFactor` 0.5), and the push with it.
    game.player.as_mut().unwrap().velocity = Vec3::ZERO;
    game.offhand = Some(ItemStack::new("minecraft:shield", 1));
    // `BlocksAttacks.blockDelayTicks` of the shield: 0.25s.
    game.using = Some((sunrise_gameplay::game::server_side::shield::OFFHAND_SLOT, 5));
    assert_eq!(
        ram(&mut game).z,
        -3.75,
        "a ram a shield blocks pushes half as far"
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
