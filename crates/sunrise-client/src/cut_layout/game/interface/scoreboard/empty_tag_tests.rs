use std::collections::HashSet;
use sunrise_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn empty_tag_selectors_follow_actual_tag_edits() {
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };
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
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    let scratch = std::env::temp_dir().join(format!(
        "sunrise-empty-tags-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    game.server.world_root = scratch.clone();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(10.5, 64.0, 0.5)).unwrap());

    game.build_registries();
    game.set_allow_commands(true);
    for (n, x, tags) in [(1, 4.5, ""), (2, 8.5, ",Tags:[\"audit\"]")] {
        assert!(game.run_command(&format!(
            "summon pig {x} 64 4.5 {{UUID:[I;267,0,0,{n}],NoAI:1b,NoGravity:1b{tags}}}"
        )));
    }
    let a = game.target_with_uuid([267, 0, 0, 1]).unwrap();
    let b = game.target_with_uuid([267, 0, 0, 2]).unwrap();
    assert_ne!(a, b);
    let select = |g: &mut sunrise_gameplay::game::ServerGame<'_>, tag: &str| {
        let text = format!("@e[type=minecraft:pig,tag={tag}]");
        let selector = sunrise_command::selector::parse(
            &mut sunrise_command::Reader::new(&text),
            &g.ui.registries,
            true,
        )
        .unwrap();
        g.select(&selector).into_iter().collect::<HashSet<_>>()
    };
    for (phase, tagged_a) in [false, true, false].into_iter().enumerate() {
        if phase != 0 {
            for (n, tagged) in [(1, tagged_a), (2, !tagged_a)] {
                assert!(game.run_command(&format!(
                    "tag 0000010b-0000-0000-0000-{n:012} {} audit",
                    if tagged { "add" } else { "remove" }
                )));
            }
        }
        assert_eq!(game.target_with_uuid([267, 0, 0, 1]), Some(a));
        assert_eq!(game.target_with_uuid([267, 0, 0, 2]), Some(b));
        assert_eq!(
            game.tags_of(a),
            if tagged_a {
                vec!["audit".to_owned()]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.tags_of(b),
            if tagged_a {
                vec![]
            } else {
                vec!["audit".to_owned()]
            }
        );
        let (empty, tagged) = if tagged_a { (b, a) } else { (a, b) };
        // Named-tag admissions pass before the old empty-tag assertion fails.
        assert_eq!(select(&mut game, "audit"), HashSet::from([tagged]));
        assert_eq!(select(&mut game, "!audit"), HashSet::from([empty]));
        assert_eq!(select(&mut game, ""), HashSet::from([empty]));
        assert_eq!(select(&mut game, "!"), HashSet::from([tagged]));
    }
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
