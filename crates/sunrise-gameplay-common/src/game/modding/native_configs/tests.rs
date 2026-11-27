use super::*;
use mod_api::ConfigKind::{Client, Common, Server};
use mod_api::{ConfigSpec, Hooks, ModEntry, Mods, Registrar, RegistrationContext};

impl NativeConfigs {
    /// Opens each registration of `kinds` in order and answers the snapshots.
    pub(super) fn load(
        &mut self,
        configs: &'static [RegisteredConfig],
        kinds: &[ConfigKind],
        config_dir: &Path,
        world: Option<&Path>,
    ) -> Vec<(&'static RegisteredConfig, ConfigSnapshot)> {
        self.started = true;
        configs
            .iter()
            .filter(|c| kinds.contains(&c.kind))
            .filter_map(|c| Some((c, self.open(c, config_dir, world)?)))
            .collect()
    }
}

pub(super) type ConfigDecl = (ConfigKind, Option<&'static str>, &'static str);

pub(super) fn with_registered_configs(
    entries: &[(&'static str, &[ConfigDecl])],
    f: impl FnOnce(&'static [RegisteredConfig]),
) {
    struct Decl(Vec<ConfigDecl>);
    #[mod_api::__subscriptions]
    impl Hooks for Decl {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for &(kind, file, default) in &self.0 {
                let mut spec = ConfigSpec::new(kind, default);
                if let Some(file) = file {
                    spec = spec.file(file);
                }
                r.register_config(spec, None).unwrap();
            }
        }
    }
    let mods: Mods = entries
        .iter()
        .map(|&(id, specs)| ModEntry::new(id, Box::new(Decl(specs.to_vec()))))
        .collect::<Vec<_>>()
        .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        f(registered_configs());
    });
}

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-native-configs-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_missing_file_gets_its_default_and_an_edited_one_is_read() {
    let root = Folder::new("default");
    let config = root.0.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("gt-common.toml"), "voltage = 512\n").unwrap();
    with_registered_configs(
        &[(
            "gt",
            &[
                (Client, None, "hud = true\n"),
                (Common, None, "voltage = 32\n"),
                (Server, None, "ores = 1\n"),
            ],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[Client, Common], &config, None);
            let got: Vec<_> = loaded
                .iter()
                .map(|(_, snapshot)| {
                    (
                        snapshot.file.as_os_str(),
                        snapshot.path.as_path(),
                        snapshot.text.as_str(),
                    )
                })
                .collect();
            assert_eq!(
                got,
                [
                    (
                        Path::new("gt-client.toml").as_os_str(),
                        config.join("gt-client.toml").as_path(),
                        "hud = true\n",
                    ),
                    (
                        Path::new("gt-common.toml").as_os_str(),
                        config.join("gt-common.toml").as_path(),
                        "voltage = 512\n",
                    ),
                ]
            );
            assert_eq!(
                fs::read_to_string(config.join("gt-client.toml")).unwrap(),
                "hud = true\n"
            );
            // Another side's kind is neither read nor written.
            assert!(!config.join("gt-server.toml").exists());
        },
    );
}

#[test]
fn a_world_overrides_a_server_config_and_the_folder_serves_the_rest() {
    let root = Folder::new("world");
    let (config, world) = (root.0.join("config"), root.0.join("saves/W"));
    fs::create_dir_all(world.join("serverconfig/gt")).unwrap();
    fs::write(world.join("serverconfig/gt/ores.toml"), "veins = 9\n").unwrap();
    with_registered_configs(
        &[(
            "gt",
            &[
                (Server, Some("gt/ores.toml"), "veins = 3\n"),
                (Server, None, "tiers = 4\n"),
                // A Common config of the world's folder is not looked for.
                (Common, None, "a = 1\n"),
            ],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[Server], &config, Some(&world));
            let got: Vec<_> = configs
                .files
                .iter()
                .map(|t| (t.snapshot.path.clone(), t.snapshot.text.as_str()))
                .collect();
            assert_eq!(loaded.len(), 2);
            assert_eq!(
                got,
                [
                    (world.join("serverconfig/gt/ores.toml"), "veins = 9\n"),
                    (config.join("gt-server.toml"), "tiers = 4\n"),
                ]
            );
            // The world's copy stands for the default: nothing is written beside it.
            assert!(!config.join("gt/ores.toml").exists());
            assert_eq!(configs.drop_server().len(), 2);
            assert!(configs.files.is_empty());
        },
    );
}

#[test]
fn registered_configs_load_in_declaration_order_and_keep_io_failures_isolated() {
    let root = Folder::new("order");
    let config = root.0.join("config");
    let broken_world = root.0.join("saves/broken");
    let clean_world = root.0.join("saves/clean");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("z-server.toml"), "fallback = true\n").unwrap();
    // An unreadable world override path (here a directory) must fail on that path,
    // never silently substitute config/z-server.toml or block other configs.
    fs::create_dir_all(broken_world.join("serverconfig/z-server.toml")).unwrap();

    // Resolved dependency order ["lib", "z", "a"] with multiple files under one owner
    // and a native path not prefixed with the mod's id.
    with_registered_configs(
        &[
            (
                "lib",
                &[
                    (Server, Some("shared/first.toml"), "first = 1\n"),
                    (Server, None, "lib = 2\n"),
                ],
            ),
            ("z", &[(Server, None, "z = 3\n")]),
            ("a", &[(Server, None, "a = 4\n")]),
        ],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[Server], &config, Some(&broken_world));
            assert_eq!(
                loaded
                    .iter()
                    .map(|(_, s)| (s.mod_id, s.file.as_os_str(), s.text.as_str()))
                    .collect::<Vec<_>>(),
                [
                    (
                        "lib",
                        Path::new("shared/first.toml").as_os_str(),
                        "first = 1\n",
                    ),
                    ("lib", Path::new("lib-server.toml").as_os_str(), "lib = 2\n"),
                    ("a", Path::new("a-server.toml").as_os_str(), "a = 4\n"),
                ]
            );
            assert_eq!(configs.drop_server().len(), 3);

            // A failed load is not latched: a later world without the broken override loads all.
            let loaded = configs.load(registered, &[Server], &config, Some(&clean_world));
            assert_eq!(
                loaded
                    .iter()
                    .map(|(_, s)| (s.mod_id, s.text.as_str()))
                    .collect::<Vec<_>>(),
                [
                    ("lib", "first = 1\n"),
                    ("lib", "lib = 2\n"),
                    ("z", "fallback = true\n"),
                    ("a", "a = 4\n"),
                ]
            );
        },
    );
}

#[test]
fn a_change_reloads_once_and_a_touch_without_one_does_not() {
    let root = Folder::new("reload");
    let config = root.0.join("config");
    with_registered_configs(&[("gt", &[(Common, None, "a = 1\n")])], |registered| {
        let mut configs = NativeConfigs::default();
        configs.load(registered, &[Common], &config, None);
        let path = config.join("gt-common.toml");
        assert!(configs.changed().is_empty());
        fs::write(&path, "a = 2\n").unwrap();
        let changed = configs.changed();
        assert_eq!(
            changed
                .iter()
                .map(|(_, s)| s.text.as_str())
                .collect::<Vec<_>>(),
            ["a = 2\n"]
        );
        assert!(configs.changed().is_empty());
        // Saved again with the same text: nothing to reload.
        fs::write(&path, "a = 2\n").unwrap();
        assert!(configs.changed().is_empty());
        // Same length, written at once after the load, as a mod's own save
        // from `ConfigEvent::Loading` is: still a change.
        fs::write(&path, "a = 3\n").unwrap();
        assert_eq!(configs.changed().len(), 1);
        fs::write(&path, "a = 2\n").unwrap();
        configs.changed();
        // Gone: the last snapshot stays, nothing fires, and recovery is not latched.
        fs::remove_file(&path).unwrap();
        assert!(configs.changed().is_empty());
        assert_eq!(configs.files[0].snapshot.text, "a = 2\n");
        fs::write(&path, "a = 4\n").unwrap();
        let recovered = configs.changed();
        assert_eq!(
            recovered
                .iter()
                .map(|(_, s)| s.text.as_str())
                .collect::<Vec<_>>(),
            ["a = 4\n"]
        );
    });
}

#[test]
fn files_are_looked_at_once_a_second_and_never_without_configs() {
    with_registered_configs(&[("gt", &[(Common, None, "")])], |registered| {
        let mut configs = NativeConfigs::default();
        let start = Instant::now();
        assert!(!configs.due(start));
        let config = &registered[0];
        configs.files.push(Tracked {
            config,
            snapshot: config.snapshot(PathBuf::new(), String::new()),
        });
        assert!(configs.due(start));
        assert!(!configs.due(start + Duration::from_millis(999)));
        assert!(configs.due(start + CHECK_EVERY));
    });
}
