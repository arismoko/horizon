//! A block-entity write that only calls `BlockEntity.setChanged`, leaving
//! `Level.sendBlockUpdated` to `block_entity_sync`, on a real mod container. The fixture
//! installs the process's block-entity type table, so the test runs in a fresh process.
use crate::{game::load_resources, settings::Settings};
use mod_api::BlockEntityRef;
use std::{path::PathBuf, sync::Arc};
use sunrise_core::nbt::Tag;
use sunrise_game_data::block_entity_types::{BlockEntityTypes, install};
use sunrise_gameplay::game::ServerGame;
use sunrise_gameplay_common::game::{
    Options, Progress, chat_common::ChatCommonGameExt1 as _, execution_role::ExecutionRole,
    native_block_entity_read::NativeBlockEntityReadGameExt1,
    native_entity_refs_common::NativeEntityRefsCommonGameExt1, test_fixtures::run_fresh_cases,
};
use sunrise_level::{chunk::Chunk, containers::BlockEntity, world::World};
use sunrise_mod_package::content::{Block, Content};
use sunrise_sim::{
    mod_content::{ContentMod, build},
    save::decode_block_entity,
};
use sunrise_world::registry::BlockStateId;

pub(super) const KEY: (i32, i32, i32) = (2, 64, 2);
pub(super) const POS: [i32; 3] = [2, 64, 2];
const BLOCK: &str = "fixture:crate";
const ENTITY: &str = "fixture:crate_entity";
pub(super) const SLOTS: usize = 27;

struct CrateMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for CrateMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key(BLOCK);
        r.block_container(
            &block,
            mod_api::ModContainer {
                slots: SLOTS,
                ..Default::default()
            },
        );
    }
}

/// A server game whose world holds the fixture crate at `POS`, its block entity loaded
/// from `{fuel:4}` with no items.
pub(super) fn with_crate(test: impl FnOnce(&mut ServerGame<'_>, BlockEntityRef)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("fixture", Box::new(CrateMod))].into();
    mod_platform::testing::with_mods(mods, || {
        let base = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::env::current_dir().unwrap();
        let root = home.join("crate-resources");
        let mut content = Content::declared("fixture");
        content
            .declare(
                &[Block {
                    id: BLOCK.into(),
                    template: "minecraft:stone".into(),
                    ..Default::default()
                }],
                &[],
            )
            .unwrap();
        build(
            &base,
            &base,
            &[ContentMod {
                id: "fixture".into(),
                content,
                jars: Vec::new(),
            }],
            &root,
            &sunrise_mod_package::enabled::EnabledMods::default(),
        )
        .unwrap();
        let (registry, _) = sunrise_world::level::data::load_registry_with_flags(&root).unwrap();
        crate::game::api_values::register_game_keys((0..registry.len()).map(|id| {
            registry
                .state(BlockStateId(id as u32))
                .unwrap()
                .name
                .clone()
        }));
        let types = BlockEntityTypes::new(
            [(BLOCK.to_owned(), ENTITY.to_owned())].into(),
            vec![ENTITY.to_owned()],
            [ENTITY.to_owned()],
        );
        install(types).unwrap();
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &base,
            &settings,
            std::time::Instant::now(),
            false,
            ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::IntegratedServer;
        let mut game = ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.world = World::new(game.res.facts().air, 1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let state = game.pack.assets.registry.default_state(BLOCK).unwrap();
        game.world.set(POS[0], POS[1], POS[2], state).unwrap();
        let saved =
            sunrise_core::nbt::parse_snbt(r#"{id:"fixture:crate_entity",x:2,y:64,z:2,fuel:4}"#)
                .unwrap()
                .0;
        let (_, entity) = decode_block_entity(&saved).unwrap();
        game.world.block_entities.insert(KEY, entity);
        if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&KEY) {
            entity.items.resize(SLOTS, None);
        }
        game.world.block_log = Some(Vec::new());
        game.server.world_root = home.join("world");
        game.build_registries();
        game.ensure_datapacks();
        game.set_allow_commands(true);
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.options.dedicated = Some(Vec::new());
        let level = game.level("minecraft:overworld").unwrap();
        let handle = game.block_entity(level, POS).unwrap();
        test(&mut game, handle);
    });
}

#[test]
#[ignore = "daily"]
fn a_quiet_write_marks_the_chunk_and_waits_for_sync() {
    if run_fresh_cases(
        "cut_layout::game::quiet_write_tests::a_quiet_write_marks_the_chunk_and_waits_for_sync",
        "SUNRISE_QUIET_WRITE_CHILD",
        &["quiet"],
    ) {
        return;
    }
    quiet_case();
}

/// A quiet write changes the saved data and marks the chunk, and no block-entity update is
/// queued for clients until `block_entity_sync`; `merge_data` queues one at once.
fn quiet_case() {
    with_crate(|game, handle| {
        let fuel = |game: &ServerGame<'_>| handle.data(game).unwrap().get(&"fuel".into()).cloned();
        game.world.modified.clear();
        assert_eq!(
            handle.merge_data_quiet(game, [("fuel".into(), Tag::Int(9))].into()),
            Ok(true)
        );
        assert_eq!(fuel(game), Some(mod_api::Tag::Int(9)));
        assert!(
            game.world.modified.contains(&(0, 0)),
            "the chunk is unsaved"
        );
        assert!(
            !game.world.block_log.as_ref().unwrap().contains(&KEY),
            "nothing is sent yet"
        );
        game.block_entity_sync(handle).unwrap();
        assert!(game.world.block_log.as_ref().unwrap().contains(&KEY));

        game.world.block_log.as_mut().unwrap().clear();
        assert_eq!(
            handle.replace_data_quiet(game, [("fuel".into(), Tag::Int(1))].into()),
            Ok(true)
        );
        assert_eq!(fuel(game), Some(mod_api::Tag::Int(1)));
        assert!(!game.world.block_log.as_ref().unwrap().contains(&KEY));
        assert_eq!(
            handle.merge_data(game, [("fuel".into(), Tag::Int(2))].into()),
            Ok(true)
        );
        assert!(game.world.block_log.as_ref().unwrap().contains(&KEY));
    });
}
