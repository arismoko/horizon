//! `/summon item` and `/summon experience_orb`: `SummonCommand.createEntity`
//! builds both from `{id:..}` through their `(EntityType, Level)` factory, so
//! the orb is never unstuck, and an empty item is discarded while loading
//! (`ItemEntity.readAdditionalSaveData`) yet the command still succeeds.
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use {
    crate::game::load_resources,
    crate::settings::Settings,
    sunrise_gameplay_common::game::{Options, Progress, execution_role},
};

pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
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
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    test(&mut game);
}

/// `ExperienceOrb(EntityType, Level)` is the factory `EntityType` uses: the
/// position is `SummonCommand`'s `snapTo` and nothing unsticks it, so an orb
/// summoned against the top of a block stays where it was asked for.
#[test]
#[ignore = "daily"]
fn a_summoned_orb_keeps_the_position_it_was_asked_for() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 100 4 stone"));
        // Its box (0.5 tall) reaches into the stone's top 0.2, which is what
        // makes the constructor's unstuck search move it in the first place.
        let prev = game.server.chat_capture.replace(Vec::new());
        let ok = game.run_command("summon minecraft:experience_orb 4.5 100.8 4.5");
        let lines = std::mem::replace(&mut game.server.chat_capture, prev).unwrap_or_default();
        assert!(ok, "{lines:?}");
        let orb = game.orbs.last().expect("an orb");
        assert_eq!(orb.position, Vec3::new(4.5, 100.8, 4.5));
        assert_eq!(orb.velocity, Vec3::ZERO);
        assert_eq!(orb.points(), 0);
        assert_eq!(orb.count, 1);
    });
}

/// Vanilla 26.3 witness: `/summon` replies with `Entity.getDisplayName` (team format, `show_entity` hover, UUID insertion, `click_event` stripped), validates `CustomName` as a text component, keeps selector `name=` plain, and `/execute summon` checks spawn bounds (#2694).
#[test]
#[ignore = "run by tools/test_summon_custom_name_parity.py"]
fn summon_replies_with_the_custom_name_of_every_kind() {
    run_summon_replies_with_the_custom_name_of_every_kind();
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::commands_server::summon_item_orb_tests::run_summon_replies_with_the_custom_name_of_every_kind;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
