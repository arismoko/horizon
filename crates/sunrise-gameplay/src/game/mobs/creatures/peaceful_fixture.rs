use crate::game::native_entity_refs::tests as fixture;
use std::sync::{Arc, OnceLock};
use sunrise_gameplay_common::game::test_fixtures::tempfile;
use sunrise_gameplay_common::game::{
    chat_common::ChatCommonGameExt1 as _, execution_role::ExecutionRole,
};
use sunrise_level::chunk::Chunk;
use sunrise_mod_package::{Content, Package, enabled::EnabledMods};
use sunrise_sim::mod_content::{self, ContentMod};

pub fn with_types(role: ExecutionRole, test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    // All cases use the same data-only fixture. Build its overlay once; never edit the base.
    static PACKAGE: OnceLock<(tempfile::TempDir, std::path::PathBuf)> = OnceLock::new();
    let (_, root) = PACKAGE.get_or_init(|| {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("peaceful.smod");
        let manifest = r#"id = "peaceful_test"
name = "Peaceful test"
version = "1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
        let rows: [(&str, &[u8]); 3] = [
            (
                "data/peaceful_test/sunrise/entity_type/trader.json",
                br#"{"template":"minecraft:wandering_trader","allowed_in_peaceful":false}"#,
            ),
            (
                "data/peaceful_test/sunrise/entity_type/zombie.json",
                br#"{"template":"minecraft:zombie","allowed_in_peaceful":true}"#,
            ),
            (
                "data/peaceful_test/sunrise/entity_type/inherited_zombie.json",
                br#"{"template":"minecraft:zombie"}"#,
            ),
        ];
        std::fs::write(&path, Package::write_files(manifest, &rows).unwrap()).unwrap();
        let package = Package::read(&path).unwrap();
        let mods = [ContentMod {
            id: "peaceful_test".into(),
            content: Content::read(&path, &package.manifest).unwrap(),
            jars: Vec::new(),
        }];
        let inventory = EnabledMods::new([("peaceful_test".into(), mod_api::Side::Both)]);
        let root = mod_content::resource_root(
            &fixture::fixture_root(),
            scratch.path(),
            &mods,
            scratch.path(),
            &inventory,
        )
        .unwrap();
        (scratch, root)
    });
    mod_api::testing::engine_platform::with_mods(Arc::from([]), || {
        fixture::with_game_role(root.clone(), role, |game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.build_registries();
            game.set_difficulty_forced(0);
            test(game);
        });
    });
}
