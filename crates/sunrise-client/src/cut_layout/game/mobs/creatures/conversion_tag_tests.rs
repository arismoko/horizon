use sunrise_core::nbt::Tag;
use {
    crate::game::load_resources, crate::settings::Settings, sunrise_gameplay_common::game::Options,
    sunrise_gameplay_common::game::Progress,
    sunrise_gameplay_common::game::commands_common::Target,
};

#[test]
#[ignore = "daily"]
fn converted_mobs_keep_command_tags_at_the_new_id_and_on_save() {
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
    game.build_registries();
    for (number, (from, to)) in [
        ("husk", "zombie"),
        ("zombie", "drowned"),
        ("skeleton", "stray"),
    ]
    .into_iter()
    .enumerate()
    {
        let uuid = [10374, 1, 0, number as i32];
        let tags = Tag::List(vec![
            Tag::String("subject".into()),
            Tag::String(from.into()),
        ]);
        let raw = Tag::compound([
            (
                "id",
                Tag::String(format!("minecraft:{from}").as_str().into()),
            ),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.5_f64.to_bits()),
                    Tag::Double(70.0_f64.to_bits()),
                    Tag::Double(0.5_f64.to_bits()),
                ]),
            ),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
            ("Tags", tags.clone()),
            ("sunrise:old_entity_only", Tag::Byte(1)),
        ]);
        let old_id = game.load_mob(raw, None).unwrap();
        assert_eq!(game.tags_of(Target::Mob(old_id)), ["subject", from]);
        let source = game.mobs.iter().find(|m| m.id == old_id).unwrap().clone();
        let new_id = game
            .convert_to(
                &source,
                &format!("minecraft:{to}"),
                sunrise_sim::mobs::conversion::Params::KEEP_ALL,
                false,
                |_, _| {},
            )
            .unwrap();
        assert_ne!(old_id, new_id);
        assert_eq!(
            game.tags_of(Target::Mob(new_id)),
            ["subject", from],
            "converted ID must keep tags"
        );
        let index = game.mobs.iter().position(|m| m.id == new_id).unwrap();
        let saved = game.mob_tree_nbt(index);
        assert_eq!(saved.get("Tags"), Some(&tags));
        assert_eq!(
            sunrise_sim::entity_nbt::kind(&saved).as_deref(),
            Some(format!("minecraft:{to}").as_str())
        );
        assert_ne!(sunrise_sim::entity_nbt::uuid(&saved), Some(uuid));
        assert!(saved.get("sunrise:old_entity_only").is_none());
        game.mobs.remove(index);
        let reloaded = game.load_mob(saved, None).unwrap();
        assert_eq!(game.tags_of(Target::Mob(reloaded)), ["subject", from]);
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
