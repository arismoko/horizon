use super::*;

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sunrise-config-storage-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn initialize_mod_runtime_for_tests() {
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    INITIALIZED.call_once(|| {
        crate::game::api_values::install_test_platform();
        mod_platform::__private::initialize()
            .expect("engine test runtime initializes before mod dispatch");
    });
}

#[test]
fn chunk_disk_uses_the_captured_native_config_root() {
    use mod_api::{
        __private::{RegistrationData, WorldgenReader, game::RegistrationContextNew},
        ChunkDiskContext, ChunkDiskKind, ChunkDiskPhase, Dist, Identifier, InstanceDirectory,
        OwningChunkDisk, OwningWorldgen, RegistrationContext, RuntimeEnvironment, WorldEpoch,
        prelude::ConfigHost,
    };
    use std::collections::BTreeMap;

    struct NoWorldgen;
    impl WorldgenReader for NoWorldgen {
        fn registry(&self, _: &str) -> Option<&[String]> {
            None
        }
        fn setting(&self, _: &str, _: &str) -> Option<&str> {
            None
        }
    }

    initialize_mod_runtime_for_tests();
    let root = ScratchDir::new("startup-host-config-dir");
    let data = RegistrationData::new(
        BTreeMap::new(),
        BTreeMap::new(),
        InstanceDirectory::from_path(&root.0).unwrap(),
        RuntimeEnvironment::new(Dist::Client, "26.3".into(), "test".into(), false),
    );

    mod_platform::__private::with_test_registration_data(data, || {
        assert_eq!(
            RegistrationContext::new()
                .instance_dir()
                .unwrap()
                .to_path_buf(),
            Some(root.0.clone())
        );
        let captured = ChunkDiskContext::new(
            WorldEpoch::new(1),
            Identifier::parse("minecraft:overworld").unwrap(),
            [0, 0],
            ChunkDiskKind::Full,
            ChunkDiskPhase::Save,
        );
        let resources = OwningWorldgen::new(&NoWorldgen, &|_, _, _| None, &|_, _| None);
        let chunk_disk = OwningChunkDisk::new(&captured, &resources, |_, _, nbt| Ok(nbt));
        assert_eq!(chunk_disk.config_dir(), root.0.join("config"));
    });
}

#[cfg(unix)]
#[test]
fn dangling_serverconfig_override_symlink_falls_through_to_base_config() {
    let root = ScratchDir::new("dangling-override");
    let config = root.0.join("config");
    let world = root.0.join("saves/W");
    let base_dangling = config.join("alpha/dangling.toml");
    let base_working = config.join("alpha/working.toml");
    let override_dangling = world.join("serverconfig/alpha/dangling.toml");
    let override_working = world.join("serverconfig/alpha/working.toml");
    let working_target = root.0.join("working-override.toml");

    fs::create_dir_all(base_dangling.parent().unwrap()).unwrap();
    fs::create_dir_all(override_dangling.parent().unwrap()).unwrap();
    fs::write(&base_dangling, "base = 1\n").unwrap();
    fs::write(&base_working, "base_working = 0\n").unwrap();
    fs::write(&working_target, "override = 1\n").unwrap();
    std::os::unix::fs::symlink(root.0.join("missing-target.toml"), &override_dangling).unwrap();
    std::os::unix::fs::symlink(&working_target, &override_working).unwrap();

    super::tests::with_registered_configs(
        &[(
            "alpha",
            &[
                (
                    ConfigKind::Server,
                    Some("alpha/dangling.toml"),
                    "default = 0\n",
                ),
                (
                    ConfigKind::Server,
                    Some("alpha/working.toml"),
                    "default = 0\n",
                ),
            ],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[ConfigKind::Server], &config, Some(&world));
            assert_eq!(loaded.len(), 2);
            assert_eq!(loaded[0].1.path, base_dangling);
            assert_eq!(loaded[0].1.text, "base = 1\n");
            assert_eq!(loaded[1].1.path, override_working);
            assert_eq!(loaded[1].1.text, "override = 1\n");

            fs::write(&base_dangling, "base = 2\n").unwrap();
            fs::write(&working_target, "override = 2\n").unwrap();
            let reloaded = configs.changed();
            assert_eq!(reloaded.len(), 2);
            assert_eq!(reloaded[0].1.path, base_dangling);
            assert_eq!(reloaded[0].1.text, "base = 2\n");
            assert_eq!(reloaded[1].1.path, override_working);
            assert_eq!(reloaded[1].1.text, "override = 2\n");
        },
    );
}

#[test]
fn native_configs_load_and_reload_follow_native_paths_and_symlinks() {
    let root = ScratchDir::new("native-paths");
    let config = root.0.join("config");
    let link_path = config.join("other/linked.toml");
    fs::create_dir_all(link_path.parent().unwrap()).unwrap();

    #[cfg(unix)]
    let target = {
        let target = root.0.join("external.toml");
        fs::write(&target, "linked = 1\n").unwrap();
        std::os::unix::fs::symlink(&target, &link_path).unwrap();
        target
    };
    #[cfg(not(unix))]
    let target = {
        fs::write(&link_path, "linked = 1\n").unwrap();
        link_path.clone()
    };

    super::tests::with_registered_configs(
        &[(
            "alpha",
            &[(
                ConfigKind::Common,
                Some("other/linked.toml"),
                "default = 0\n",
            )],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[ConfigKind::Common], &config, None);
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].1.path, link_path);
            assert_eq!(loaded[0].1.text, "linked = 1\n");

            fs::write(&target, "linked = 2\n").unwrap();
            let reloaded = configs.changed();
            assert_eq!(reloaded.len(), 1);
            assert_eq!(reloaded[0].1.path, link_path);
            assert_eq!(reloaded[0].1.text, "linked = 2\n");
        },
    );
}
