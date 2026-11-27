use mod_api::PackFinders;
use mod_api::{AddPackFindersEvent, Hooks, Identifier, ModEntry};
use mod_platform::testing::with_mods;
use std::{fs, io::Write, path::PathBuf};
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::pack_discovery_client::into_resource_pack;
#[cfg(feature = "client")]
use sunrise_client_render::packs;
#[cfg(feature = "client")]
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::pack_discovery::{
    HookId, ModsState, PackDiscoveryContext, PackFinderError, PackPosition, PackRegistration,
    PackSource, PackType, Registry, StartupDiscovery, data_packs, discover,
};
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::pack_discovery::{ResourceSource, Source, data_repository};
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::pack_finders_common::CommonGameExt1;
use sunrise_level::datapack::FunctionLine;
#[cfg(feature = "client")]
use sunrise_level::datapack::{DataPacks, DataStore};
#[cfg(feature = "client")]
use sunrise_level::pack_meta::{RESOURCE_FORMAT, SelectionModel};

const ASSET_PACK: &str = "assets/packmod/resourcepacks/optional";
const DATA_PACK: &str = "data/packmod/datapacks/optional";
const ASSET: &str = "assets/packmod/lang/en_us.json";

struct Fixture {
    pub root: PathBuf,
    package: PathBuf,
    archive: bool,
    files: Vec<(String, Vec<u8>)>,
}
impl Fixture {
    fn new(archive: bool) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pack-finders-{}-{}-{number}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&root).unwrap();
        let metadata = |format: (i64, i64)| {
            serde_json::to_vec(&serde_json::json!({
                "pack": {"description": {"translate": "packmod.description"},
                    "min_format": [format.0, format.1], "max_format": [format.0, format.1]},
                "filter": {"block": [{"namespace": "packmod", "path": "lang/hidden\\.json"}]},
            }))
            .unwrap()
        };
        let package = root.join(if archive { "packmod.smod" } else { "packmod" });
        let fixture = Self {
            root, package, archive,
            files: vec![
                ("sunrise-mod.toml".into(), b"id='packmod'\nname='Pack mod'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n".to_vec()),
                (ASSET.into(), br#"{"packmod.optional.title":"Optional example","packmod.description":"An optional test pack"}"#.to_vec()),
                (format!("{ASSET_PACK}/pack.mcmeta"), metadata(sunrise_core::version::RESOURCE_PACK_FORMAT)),
                (format!("{ASSET_PACK}/{ASSET}"), br#"{"packmod.optional":"first"}"#.to_vec()),
                (format!("{DATA_PACK}/pack.mcmeta"), metadata(sunrise_core::version::DATA_PACK_FORMAT)),
                (format!("{DATA_PACK}/data/packmod/function/probe.mcfunction"), b"say from-optional\n".to_vec()),
            ],
        };
        fixture.publish();
        fixture
    }
    fn publish(&self) {
        if self.archive {
            let next = self.package.with_extension("next.smod");
            let mut zip = zip::ZipWriter::new(fs::File::create(&next).unwrap());
            for (name, bytes) in &self.files {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            fs::rename(next, &self.package).unwrap();
        } else {
            for (name, bytes) in &self.files {
                let path = self.package.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, bytes).unwrap();
            }
        }
    }
    fn sources(&self) -> ModsState {
        ModsState {
            selected: vec![sunrise_mod_package::selection::SelectedPackage {
                path: self.package.clone(),
                package: sunrise_mod_package::Package::read(&self.package).unwrap(),
            }]
            .into(),
            ..Default::default()
        }
    }
    fn asset_id(&self) -> String {
        format!("mod/packmod:{ASSET_PACK}")
    }
    fn data_id(&self) -> String {
        format!("mod/packmod:{DATA_PACK}")
    }
    fn replace_asset(&mut self) {
        self.files
            .iter_mut()
            .find(|(name, _)| name == &format!("{ASSET_PACK}/{ASSET}"))
            .unwrap()
            .1 = br#"{"packmod.optional":"second"}"#.to_vec();
        self.publish();
    }
    fn add_feature_flag(&mut self) {
        let file = self
            .files
            .iter_mut()
            .find(|(name, _)| name == &format!("{DATA_PACK}/pack.mcmeta"))
            .unwrap();
        let mut meta: serde_json::Value = serde_json::from_slice(&file.1).unwrap();
        meta["features"] = serde_json::json!({"enabled": ["minecraft:trade_rebalance"]});
        file.1 = serde_json::to_vec(&meta).unwrap();
        self.publish();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Finder {
    pub feature: bool,
    required: bool,
    position: PackPosition,
    hidden: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Finder {
    fn add_pack_finders(
        &self,
        _context: &mut dyn PackDiscoveryContext,
        event: &mut AddPackFindersEvent<'_>,
    ) {
        for (kind, path) in [
            (PackType::ClientResources, ASSET_PACK),
            (PackType::ServerData, DATA_PACK),
        ] {
            let mut pack = PackRegistration::new(
                Identifier::parse(&format!("packmod:{path}")).unwrap(),
                kind,
                mod_api::Text::translate("packmod.optional.title", vec![]),
            );
            pack.always_active = self.required;
            pack.position = self.position;
            pack.hidden = self.hidden;
            if self.feature && kind == PackType::ServerData {
                pack.source = PackSource::Feature;
            }
            event.packs.add_pack_finders(pack).unwrap();
        }
    }
}
fn with_finder(test: impl FnOnce()) {
    with_finder_config(false, false, test);
}
fn with_finder_config(feature: bool, required: bool, test: impl FnOnce()) {
    with_finder_hidden(feature, required, false, test);
}
fn with_finder_hidden(feature: bool, required: bool, hidden: bool, test: impl FnOnce()) {
    with_mods(
        vec![ModEntry::new(
            "packmod",
            Box::new(Finder {
                feature,
                required,
                position: PackPosition::Bottom,
                hidden,
            }),
        )]
        .into(),
        test,
    );
}

#[test]
fn both_startup_repositories_lend_only_available_text_services() {
    use std::sync::{Arc, Mutex};

    struct Observe(Arc<Mutex<Vec<PackType>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn add_pack_finders(
            &self,
            context: &mut dyn PackDiscoveryContext,
            event: &mut AddPackFindersEvent<'_>,
        ) {
            let component = mod_api::Text::literal("Startup");
            let fallback = mod_api::Text::new(mod_api::text::TextContent::Translatable {
                key: "test.discovery.missing".into(),
                fallback: Some("Fallback %s".into()),
                arguments: vec![mod_api::Text::literal("Pack")],
            });
            let mut deep = mod_api::Text::literal("Deep");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                deep = mod_api::Text::literal("").append(deep);
            }
            assert_eq!(context.text_string(&component), "Startup");
            assert_eq!(context.text_string(&fallback), "Fallback Pack");
            assert_eq!(context.text_string(&deep), "Deep");
            assert_eq!(
                context.text_from_json(r#""Startup""#),
                Err(mod_api::text::TextFormError::Unavailable)
            );
            assert!(context.client_text().is_none());
            assert!(context.live_host().is_none());
            self.0.lock().unwrap().push(event.packs.pack_type());
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new(
            "discovery_text",
            Box::new(Observe(seen.clone())),
        )]
        .into(),
        || {
            let sources = ModsState::default();
            assert!(discover(PackType::ClientResources, &sources).is_empty());
            assert!(data_packs(&sources).is_empty());
        },
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [PackType::ClientResources, PackType::ServerData]
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn live_pack_rediscovery_can_read_a_registered_client_setting() {
    use mod_api::{
        HostSide, Registrar, RegistrationContext, SettingKey, SettingSpec, SettingValue,
    };
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::{record_key, run_fresh_cases},
        sunrise_gameplay_common::game::native_hooks::mod_settings::ModSettingsGameExt as _,
    };

    if run_fresh_cases(
        "cut_layout::game::pack_discovery::tests::live_pack_rediscovery_can_read_a_registered_client_setting",
        "SUNRISE_PACK_DISCOVERY_CHILD",
        &["client"],
    ) {
        return;
    }

    struct SettingFinder(std::sync::OnceLock<SettingKey>);
    #[mod_api::__subscriptions]
    impl Hooks for SettingFinder {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            record_key(
                &self.0,
                registrar
                    .register_setting(SettingSpec::toggle("offer", "packmod.offer", false))
                    .unwrap(),
            );
        }

        fn add_pack_finders(
            &self,
            context: &mut dyn PackDiscoveryContext,
            event: &mut AddPackFindersEvent<'_>,
        ) {
            if let Some(HostSide::Client(host)) = context.live_host()
                && host.setting_bool(*self.0.get().unwrap()) == Ok(true)
            {
                event
                    .packs
                    .add_pack_finders(PackRegistration::new(
                        Identifier::parse(&format!("packmod:{ASSET_PACK}")).unwrap(),
                        PackType::ClientResources,
                        mod_api::Text::literal("Setting-selected pack"),
                    ))
                    .unwrap();
            }
        }
    }

    let fixture = Fixture::new(false);
    with_mods(
        vec![ModEntry::new(
            "packmod",
            Box::new(SettingFinder(Default::default())),
        )]
        .into(),
        || {
            with_client_game(|game| {
                game.mods = fixture.sources();
                let ty = PackType::ClientResources;
                assert!(game.discover_mod_packs(ty).is_empty());
                let key = mod_api::__private::setting_key_by_name("packmod", "offer").unwrap();
                assert_eq!(game.set_setting(key, SettingValue::Toggle(true)), Ok(true));
                let packs = game.discover_mod_packs(ty);
                assert_eq!(packs.len(), 1);
                assert_eq!(packs[0].meta.id, fixture.asset_id());
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
fn hidden_packs_reach_the_screens_marked_hidden_and_still_apply() {
    let fixture = Fixture::new(false);
    let sources = fixture.sources();
    with_finder_hidden(false, true, true, || {
        let packs: Vec<_> = discover(PackType::ClientResources, &sources)
            .into_iter()
            .map(into_resource_pack)
            .collect();
        let infos = sunrise_client_gameplay::game::client_side::packs::pack_infos(
            &fixture.root,
            &[],
            &packs,
        );
        let info = infos
            .iter()
            .find(|info| info.id == fixture.asset_id())
            .unwrap();
        assert!(info.hidden && info.required);
        let data = data_packs(&sources);
        assert!(data[0].selection.as_ref().is_some_and(|info| info.hidden));
        // Required and hidden: on without the player picking it.
        let base = fixture.root.join("base");
        fs::create_dir_all(base.join("assets/packmod/lang")).unwrap();
        fs::write(base.join(ASSET), b"base").unwrap();
        let on = packs::resources(
            ResourceSource::directory(base),
            &packs::enabled(&packs, &[fixture.asset_id()]),
        )
        .unwrap();
        assert_eq!(on.read(ASSET).unwrap(), br#"{"packmod.optional":"first"}"#);
    });
}

#[cfg(feature = "client")]
#[test]
fn contributed_packs_are_discoverable_and_only_selected_resources_apply() {
    for archive in [false, true] {
        let fixture = Fixture::new(archive);
        let sources = fixture.sources();
        with_finder(|| {
            let packs: Vec<_> = discover(PackType::ClientResources, &sources)
                .into_iter()
                .map(into_resource_pack)
                .collect();
            assert_eq!(packs.len(), 1);
            let infos = sunrise_client_gameplay::game::client_side::packs::pack_infos(
                &fixture.root,
                &[],
                &packs,
            );
            assert!(
                infos
                    .iter()
                    .any(|info| info.id == fixture.asset_id() && !info.required)
            );
            let base = fixture.root.join("base");
            fs::create_dir_all(base.join("assets/packmod/lang")).unwrap();
            fs::write(base.join(ASSET), b"base").unwrap();
            fs::write(base.join("assets/packmod/lang/hidden.json"), b"hidden").unwrap();
            let base = ResourceSource::directory(base);
            let off = packs::resources(base.clone(), &packs::enabled(&packs, &[])).unwrap();
            assert_eq!(off.read(ASSET).unwrap(), b"base");
            let selected = packs::enabled(&packs, &[fixture.asset_id()]);
            let on = packs::resources(base, &selected).unwrap();
            assert_eq!(on.read(ASSET).unwrap(), br#"{"packmod.optional":"first"}"#);
            assert!(on.read("assets/packmod/lang/hidden.json").is_err());

            let data = data_packs(&sources);
            assert_eq!(data.len(), 1);
            let world = fixture.root.join("world");
            fs::create_dir(&world).unwrap();
            let repository = data_repository(&world, &fixture.root, &data);
            assert!(sunrise_level::datapack::enabled_ids(&world, &repository, &[]).is_empty());
            fs::write(
                world.join("datapacks.json"),
                serde_json::json!({
                    "enabled": [fixture.data_id()], "disabled": [],
                })
                .to_string(),
            )
            .unwrap();
            let stack = sunrise_gameplay::game::functions::pack_stack(
                &world,
                &fixture.root,
                &Default::default(),
                &Default::default(),
                true,
                &[],
                &data,
            );
            let enabled = stack.enabled_packs().collect::<Vec<_>>();
            let store = DataStore::load(&DataPacks::new(&enabled));
            assert_eq!(
                store.functions["packmod:probe"].lines,
                [FunctionLine::Command("say from-optional".into())]
            );
            if archive {
                assert!(!fixture.root.join("packmod").exists());
            }
        });
    }
}

#[test]
fn rediscovery_opens_the_new_archive_without_changing_old_readers() {
    let mut fixture = Fixture::new(true);
    let sources = fixture.sources();
    with_finder(|| {
        let old = discover(PackType::ClientResources, &sources);
        fixture.replace_asset();
        let new = discover(PackType::ClientResources, &sources);
        assert_eq!(
            old[0].meta.source.open().unwrap().read(ASSET).unwrap(),
            br#"{"packmod.optional":"first"}"#
        );
        assert_eq!(
            new[0].meta.source.open().unwrap().read(ASSET).unwrap(),
            br#"{"packmod.optional":"second"}"#
        );
        assert!(!fixture.root.join("packmod").exists());
    });
}

#[test]
fn refused_registrations_leave_the_repository_unchanged() {
    let fixture = Fixture::new(false);
    let mut registry = Registry::new(PackType::ClientResources, fixture.sources());
    let entry = |path: &str| {
        PackRegistration::new(
            Identifier::parse(path).unwrap(),
            PackType::ClientResources,
            mod_api::Text::translate("packmod.optional.title", vec![]),
        )
    };
    let valid = format!("packmod:{ASSET_PACK}");
    let mut titled = entry(&valid);
    titled.title = mod_api::Text::literal("leaf");
    for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
        titled.title = mod_api::Text::literal("").append(titled.title);
    }
    registry.add_pack_finders(titled).unwrap();
    assert_eq!(registry.packs[0].meta.title.plain(), "leaf");
    assert_eq!(registry.packs.len(), 1);
    for (path, error) in [
        (valid.as_str(), PackFinderError::Duplicate),
        ("packmod:../outside", PackFinderError::InvalidPath),
        ("absent:optional", PackFinderError::MissingPackage),
        ("packmod:missing", PackFinderError::InvalidMetadata),
    ] {
        assert_eq!(registry.add_pack_finders(entry(path)), Err(error));
        assert_eq!(registry.packs.len(), 1);
    }
    assert_eq!(
        registry.packs[0]
            .meta
            .source
            .open()
            .unwrap()
            .read(ASSET)
            .unwrap(),
        br#"{"packmod.optional":"first"}"#
    );
}

#[cfg(feature = "client")]
#[test]
fn required_packs_keep_their_priority_and_cannot_be_cleared() {
    let fixture = Fixture::new(false);
    let sources = fixture.sources();
    with_finder_config(false, true, || {
        let mut all: Vec<_> = discover(PackType::ClientResources, &sources)
            .into_iter()
            .map(into_resource_pack)
            .collect();
        let optional = fixture.root.join("override");
        fs::create_dir_all(optional.join("assets/packmod/lang")).unwrap();
        fs::write(optional.join(ASSET), br#"{"packmod.optional":"override"}"#).unwrap();
        fs::write(
            optional.join("pack.mcmeta"),
            &fixture
                .files
                .iter()
                .find(|(name, _)| name == &format!("{ASSET_PACK}/pack.mcmeta"))
                .unwrap()
                .1,
        )
        .unwrap();
        all.push(
            packs::pack_for(
                "file/override".into(),
                Text::translate("packmod.optional.title", &[]),
                Source::Dir(optional),
                RESOURCE_FORMAT,
            )
            .unwrap(),
        );
        let empty = ResourceSource::directory(fixture.root.join("base"));
        let required = packs::enabled(&all, &[]);
        assert!(!packs::has_optional_pack(&required));
        assert_eq!(
            packs::resources(empty.clone(), &required)
                .unwrap()
                .read(ASSET)
                .unwrap(),
            br#"{"packmod.optional":"first"}"#
        );
        let selected = packs::enabled(&all, &["file/override".to_owned()]);
        assert!(packs::has_optional_pack(&selected));
        assert_eq!(
            packs::resources(empty, &selected)
                .unwrap()
                .read(ASSET)
                .unwrap(),
            br#"{"packmod.optional":"override"}"#
        );
        let info = all[0].selection.as_ref().unwrap();
        let model = SelectionModel::new(vec![info.selectable()], &[]);
        assert!(!model.can_unselect(true, 0));

        let mut second_required = all[0].clone();
        second_required.id = "mod/second".into();
        second_required.selection.as_mut().unwrap().id = second_required.id.clone();
        all.push(second_required);
        let order = |ids: &[String]| {
            packs::enabled(&all, ids)
                .into_iter()
                .map(|pack| pack.id)
                .collect::<Vec<_>>()
        };
        let mut reordered = order(&[]);
        assert_eq!(reordered.len(), 2);
        reordered.reverse();
        assert_eq!(order(&reordered), reordered);
        assert!(!packs::has_optional_pack(&packs::enabled(&all, &reordered)));
        reordered.push("file/override".into());
        assert!(packs::has_optional_pack(&packs::enabled(&all, &reordered)));

        let data = data_packs(&sources);
        let mut options = sunrise_sim::world_options::WorldOptions::default();
        options.data_packs.push("file/already_installed".into());
        options.set_additional_data_packs(data.clone().into());
        assert!(options.data_packs.contains(&fixture.data_id()));
        assert!(
            options
                .data_packs
                .contains(&"file/already_installed".into())
        );
        assert!(options.data_packs.contains(&"vanilla".into()));
        let world = fixture.root.join("world");
        let repository = data_repository(&world, &fixture.root, &data);
        assert!(
            sunrise_level::datapack::enabled_ids(&world, &repository, &[])
                .contains(&fixture.data_id())
        );
    });
}

#[test]
fn contributed_experiments_supply_the_chosen_world_flags() {
    let mut fixture = Fixture::new(true);
    fixture.add_feature_flag();
    with_finder_config(true, false, || {
        let data = data_packs(&fixture.sources());
        let features = sunrise_level::datapack::feature_repository(&fixture.root, &data);
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].id, fixture.data_id());
        let mut options = sunrise_sim::world_options::WorldOptions::default();
        options.set_additional_data_packs(data.into());
        assert_eq!(
            options.enabled_features(&fixture.root),
            ["minecraft:vanilla"]
        );
        options.data_packs.push(fixture.data_id());
        assert_eq!(
            options.enabled_features(&fixture.root),
            ["minecraft:trade_rebalance", "minecraft:vanilla"]
        );
    });
}

#[test]
#[ignore = "daily"]
fn registered_data_commands_respect_flags_required_packs_and_default_priority() {
    let mut ran = 0;
    for (feature, required) in [(false, false), (false, true), (true, false)] {
        let mut fixture = Fixture::new(false);
        if feature {
            fixture.add_feature_flag();
        }
        with_finder_config(feature, required, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                ran += 1;
                let world = fixture.root.join("world");
                let user = world.join("datapacks/user");
                fs::create_dir_all(user.join("data/packmod/function")).unwrap();
                fs::write(
                    user.join("pack.mcmeta"),
                    br#"{"pack":{"description":"User override fixture"}}"#,
                )
                .unwrap();
                fs::write(
                    user.join("data/packmod/function/probe.mcfunction"),
                    b"say from-user\n",
                )
                .unwrap();
                game.server.world_root = world;
                game.mods = fixture.sources();
                game.set_allow_commands(true);
                game.reload_datapacks(false, &[]).unwrap();
                assert!(game.run_command("datapack list"));
                let enable = format!("datapack enable \"{}\"", fixture.data_id());
                let disable = format!("datapack disable \"{}\"", fixture.data_id());
                if feature {
                    assert!(!game.run_command(&enable));
                    assert!(!game.server.fns.0.enabled().contains(&fixture.data_id()));
                    let flags = vec![
                        "minecraft:vanilla".into(),
                        "minecraft:trade_rebalance".into(),
                    ];
                    game.level.enabled_features = flags.clone().into();
                    game.reload_datapacks(false, &flags).unwrap();
                    assert!(game.server.fns.0.enabled().contains(&fixture.data_id()));
                    assert!(!game.run_command(&disable));
                    assert!(game.server.fns.0.enabled().contains(&fixture.data_id()));
                } else if required {
                    assert!(game.server.fns.0.enabled().contains(&fixture.data_id()));
                    assert!(!game.run_command(&disable));
                    assert!(game.server.fns.0.enabled().contains(&fixture.data_id()));
                } else {
                    assert!(!game.server.fns.0.enabled().contains(&fixture.data_id()));
                    assert!(game.run_command(&enable));
                    assert_eq!(
                        game.server.fns.0.store.functions["packmod:probe"].lines,
                        [FunctionLine::Command("say from-user".into())]
                    );
                    assert!(game.run_command(&disable));
                    assert!(game.run_command(&format!("{enable} last")));
                    assert_eq!(
                        game.server.fns.0.store.functions["packmod:probe"].lines,
                        [FunctionLine::Command("say from-optional".into())]
                    );
                }
            });
        });
    }
    assert_eq!(ran, 3, "every prepared-resource case must run");
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_mods_pack_is_on_the_screen_and_applies_through_client_reload() {
    // Reload publishes a language just as the other game fixtures do. Keep this
    // fixture's publications visible locally, not at the mercy of their reloads.
    let _language = sunrise_game_data::lang::isolate_publication(
        sunrise_game_data::lang::Language::of("en_us", []),
    );
    let mut fixture = Fixture::new(true);
    let mut ran = false;
    with_finder(|| {
        crate::game::server_side::native_entity_refs::tests::with_uncached_client_game(|game| {
            ran = true;
            game.mods = fixture.sources();
            let staged = sunrise_client_gameplay::game::client_side::packs::stage_pack(
                &game.vanilla_root(),
                &[],
                &[],
                &game.mods,
                game.register_client_reload_listeners(
                    &game
                        .capture_candidate_settings(&game.options.resources)
                        .unwrap(),
                ),
                &[],
                &game.pack.assets,
                &game.res,
                game.client_res(),
                &game.client.client_pack.context,
                false,
                false,
                &sunrise_gameplay_common::game::Progress::new(),
            )
            .unwrap();
            game.apply_staged_pack(staged).unwrap();
            assert_eq!(
                sunrise_game_data::lang::get("packmod.optional.title"),
                "Optional example"
            );
            assert_eq!(
                sunrise_game_data::lang::get("packmod.optional"),
                "packmod.optional"
            );
            game.refresh_pack_lists();
            assert!(
                game.chat_ui_for_test()
                    .options
                    .pack_infos
                    .iter()
                    .any(|info| info.id == fixture.asset_id())
            );
            game.client.settings.resource_packs = vec![fixture.asset_id()];
            for expected in ["first", "second"] {
                let listeners = game.register_client_reload_listeners(
                    &game
                        .capture_candidate_settings(&game.options.resources)
                        .unwrap(),
                );
                let staged = sunrise_client_gameplay::game::client_side::packs::stage_pack(
                    &game.vanilla_root(),
                    &game.client.settings.resource_packs,
                    &[],
                    &game.mods,
                    listeners,
                    &[],
                    &game.pack.assets,
                    &game.res,
                    game.client_res(),
                    &game.client.client_pack.context,
                    false,
                    false,
                    &sunrise_gameplay_common::game::Progress::new(),
                )
                .unwrap();
                assert_eq!(staged.language.get("packmod.optional"), Some(expected));
                game.apply_staged_pack(staged).unwrap();
                assert_eq!(sunrise_game_data::lang::get("packmod.optional"), expected);
                assert!(
                    !game
                        .options
                        .resources
                        .join(format!("{ASSET_PACK}/{ASSET}"))
                        .exists()
                );
                if expected == "first" {
                    fixture.replace_asset();
                }
            }
        });
    });
    assert!(ran, "the prepared-resource fixture must run this case");
}

#[cfg(feature = "client")]
#[test]
fn hidden_optional_data_pack_is_command_selectable_and_loads_its_function() {
    let fixture = Fixture::new(false);
    with_finder_hidden(false, false, true, || {
        let data = data_packs(&fixture.sources());
        assert_eq!(data.len(), 1);
        let mut info = data[0].selection.clone().unwrap();
        assert!(info.hidden);
        assert!(!info.required);
        info.title = Text::literal("Optional data fixture");
        let atlas = sunrise_client::hud::HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let m = sunrise_client::screen_draw::Measure(&atlas.font);
        let drawn_labels = |info| {
            let screen = sunrise_client::pack_screen::PackScreen::new(
                &m,
                Text::literal("Data packs"),
                480,
                320,
                false,
                vec![info],
                &[],
            );
            screen
                .draw(&m, (0, 0), 0, 0)
                .into_iter()
                .filter_map(|draw| {
                    if let sunrise_client::screen_draw::Draw::Text { text, .. } = draw {
                        Some(text.plain())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert!(
            !drawn_labels(info.clone())
                .iter()
                .any(|s| s == "Optional data fixture")
        );
        info.hidden = false;
        assert!(
            drawn_labels(info)
                .iter()
                .any(|s| s == "Optional data fixture")
        );

        let world = fixture.root.join("world");
        fs::create_dir(&world).unwrap();
        let repository = data_repository(&world, &fixture.root, &data);
        assert!(
            repository
                .iter()
                .any(|p| p.id == "mod/packmod:data/packmod/datapacks/optional")
        );
        assert!(sunrise_level::datapack::enabled_ids(&world, &repository, &[]).is_empty());
        fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["mod/packmod:data/packmod/datapacks/optional"],"disabled":[]}"#,
        )
        .unwrap();
        let stack = sunrise_gameplay::game::functions::pack_stack(
            &world,
            &fixture.root,
            &Default::default(),
            &Default::default(),
            true,
            &[],
            &data,
        );
        let enabled = stack.enabled_packs().collect::<Vec<_>>();
        let store = DataStore::load(&DataPacks::new(&enabled));
        assert_eq!(
            store.functions["packmod:probe"].lines,
            [FunctionLine::Command("say from-optional".into())]
        );
    });
}

#[test]
fn a_panicking_pack_finder_does_not_reserve_its_pack_id() {
    struct FailingFinder(Finder);
    #[mod_api::__subscriptions]
    impl Hooks for FailingFinder {
        fn add_pack_finders(
            &self,
            context: &mut dyn PackDiscoveryContext,
            event: &mut AddPackFindersEvent<'_>,
        ) {
            self.0.add_pack_finders(context, event);
            panic!("finder failed after offering its pack");
        }
    }
    let fixture = Fixture::new(false);
    let sources = fixture.sources();
    let finder = |required| Finder {
        feature: false,
        required,
        position: PackPosition::Bottom,
        hidden: false,
    };
    with_mods(
        vec![
            ModEntry::new("failing", Box::new(FailingFinder(finder(true)))),
            ModEntry::new("healthy", Box::new(finder(false))),
        ]
        .into(),
        || {
            for _ in 0..2 {
                let packs = discover(PackType::ClientResources, &sources);
                assert_eq!(packs.len(), 1);
                assert_eq!(packs[0].meta.id, fixture.asset_id());
                assert!(!packs[0].meta.selection.as_ref().unwrap().required);
            }
            assert_eq!(
                mod_api::__private::poisoned_listeners(HookId::AddPackFinders),
                1
            );
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "failing",
                    hook: "add_pack_finders",
                }]
            );
        },
    );
}

#[test]
fn disabled_native_mods_skip_add_pack_finders_at_startup_and_after_toggle() {
    use std::collections::BTreeSet;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Probe {
        calls: Arc<AtomicUsize>,
        register_packs: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn add_pack_finders(
            &self,
            _context: &mut dyn PackDiscoveryContext,
            event: &mut AddPackFindersEvent<'_>,
        ) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.register_packs {
                return;
            }
            let path = match event.packs.pack_type() {
                PackType::ClientResources => ASSET_PACK,
                PackType::ServerData => DATA_PACK,
                _ => return,
            };
            let pack = PackRegistration::new(
                Identifier::parse(&format!("packmod:{path}")).unwrap(),
                event.packs.pack_type(),
                mod_api::Text::translate("packmod.optional.title", vec![]),
            );
            event.packs.add_pack_finders(pack).unwrap();
        }
    }

    let fixture = Fixture::new(false);
    let disabled_calls = Arc::new(AtomicUsize::new(0));
    let enabled_calls = Arc::new(AtomicUsize::new(0));

    with_mods(
        vec![
            ModEntry::new(
                "discovery_probe",
                Box::new(Probe {
                    calls: disabled_calls.clone(),
                    register_packs: false,
                }),
            ),
            ModEntry::new(
                "packmod",
                Box::new(Probe {
                    calls: enabled_calls.clone(),
                    register_packs: true,
                }),
            ),
        ]
        .into(),
        || {
            // Startup before Game::start_mods applies saved switches to runtime dispatch:
            // sunrise-mods.txt has disabled `discovery_probe`, while its runtime slot is still on.
            let mut sources = fixture.sources();
            let disabled = BTreeSet::from(["discovery_probe".to_owned()]);
            sources.capture_code_switches(&disabled);
            assert!(mod_api::__private::mod_index_on(
                mod_api::__private::mod_index("discovery_probe").unwrap()
            ));

            let client_packs = discover(PackType::ClientResources, &sources);
            let server_packs = data_packs(&sources);
            assert_eq!(disabled_calls.load(Ordering::SeqCst), 0);
            assert_eq!(enabled_calls.load(Ordering::SeqCst), 2);
            assert_eq!(client_packs.len(), 1);
            assert_eq!(client_packs[0].meta.id, fixture.asset_id());
            assert_eq!(server_packs.len(), 1);
            assert_eq!(server_packs[0].id, fixture.data_id());

            // After Game::start_mods applies switches, both startup-style and direct dispatch skip it.
            sources.apply_code_switches(&disabled);
            assert!(!mod_api::__private::mod_index_on(
                mod_api::__private::mod_index("discovery_probe").unwrap()
            ));
            let client_packs = discover(PackType::ClientResources, &sources);
            let server_packs = data_packs(&sources);
            assert_eq!(disabled_calls.load(Ordering::SeqCst), 0);
            assert_eq!(enabled_calls.load(Ordering::SeqCst), 4);
            assert_eq!(client_packs.len(), 1);
            assert_eq!(server_packs.len(), 1);

            let mut registry = Registry::new(PackType::ClientResources, sources.clone());
            mod_api::dispatch_add_pack_finders(&mut StartupDiscovery, |listen| {
                registry.register_listener(listen)
            });
            assert_eq!(disabled_calls.load(Ordering::SeqCst), 0);
            assert_eq!(enabled_calls.load(Ordering::SeqCst), 5);
            assert_eq!(registry.packs.len(), 1);

            // Live toggles flip which listener runs without invoking the newly disabled mod.
            assert!(mod_api::__private::set_mod_enabled("discovery_probe", true));
            assert!(mod_api::__private::set_mod_enabled("packmod", false));
            assert!(discover(PackType::ClientResources, &sources).is_empty());
            assert!(data_packs(&sources).is_empty());
            assert_eq!(disabled_calls.load(Ordering::SeqCst), 2);
            assert_eq!(enabled_calls.load(Ordering::SeqCst), 5);
        },
    );
}
