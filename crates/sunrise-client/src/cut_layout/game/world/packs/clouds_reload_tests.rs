//! A selected pack whose cloud texture vanilla 26.3 reads past (`CloudRenderer.prepare` throws
//! `(5, 2) outside of image bounds (4, 6)`): the whole load fails as vanilla's does
//! (`Minecraft.rollbackResourcePacks`, `clearResourcePacksOnError`), on a reload and at startup,
//! and the pack's other overrides go with it. So does a pack whose `core/clouds` one of the
//! pipelines vanilla requires can't be made from (`ShaderManager.apply`'s "Failed to load
//! required shader programs"), even with Improved Transparency off. Each case runs in a process
//! of its own, in a folder whose `resourcepacks` holds the packs.
use sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

const NAME: &str = "cut_layout::game::client_side::packs::clouds_reload_tests::a_pack_whose_clouds_vanilla_reads_past_fails_the_load";
const CASE: &str = "SUNRISE_CLOUDS_RELOAD_CASE";

/// A `w` by `h` RGBA image with the cells at `filled` opaque.
fn png(w: u32, h: u32, filled: &[[u32; 2]]) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for [x, y] in filled {
        rgba[((y * w + x) * 4 + 3) as usize] = 255;
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgba).unwrap();
    writer.finish().unwrap();
    out
}

/// `resourcepacks/<name>`: a pack with its own `core/sky` (a visible override, which the atlas
/// loads) and a 4 by 6 cloud texture with `cells` filled.
pub(super) fn write_pack(name: &str, cells: &[[u32; 2]]) {
    write_pack_with(name, cells, &[]);
}

/// [`write_pack`] with `own` files besides.
fn write_pack_with(name: &str, cells: &[[u32; 2]], own: &[(&str, &str)]) {
    let dir = std::env::current_dir()
        .unwrap()
        .join("resourcepacks")
        .join(name);
    let file = |path: &str, bytes: &[u8]| {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    file(
        "pack.mcmeta",
        br#"{"pack": {"description": "clouds", "pack_format": 97, "min_format": [97, 0], "max_format": [97, 1]}}"#,
    );
    file(
        "assets/minecraft/shaders/core/sky.vsh",
        b"#version 330\nlayout(location = 0) in vec3 Position;\n\
          void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n",
    );
    file(
        "assets/minecraft/shaders/core/sky.fsh",
        b"#version 330\nlayout(location = 0) out vec4 fragColor;\n\
          void main() {\n    fragColor = vec4(1.0, 0.0, 0.0, 1.0);\n}\n",
    );
    file(
        "assets/minecraft/textures/environment/clouds.png",
        &png(4, 6, cells),
    );
    for (path, text) in own {
        file(path, text.as_bytes());
    }
}

/// A `clouds.fsh` (for vanilla's `clouds.vsh`) only the transmittance stage of `OIT_CLOUDS`
/// can't use: there it reads a block no clouds pipeline gives.
const OIT_ONLY_BROKEN_FSH: &str = "#version 330\nlayout(location = 0) in float vertexDistance;\n\
    layout(location = 1) in vec4 vertexColor;\n\
    layout(std140) uniform Other {\n    vec4 Tint;\n};\nlayout(location = 0) out vec4 fragColor;\n\
    void main() {\n    fragColor = vertexColor;\n#ifdef OIT_TRANSMITTANCE\n    fragColor *= Tint;\n#endif\n}\n";

/// A tall texture vanilla reads inside (`tall_inner` of the Java comparison) and one with a cell
/// in its first column, whose west neighbour vanilla reads at (5, 2).
pub(super) const INNER: &[[u32; 2]] = &[[1, 0], [2, 1], [1, 3], [2, 5]];
const PAST: &[[u32; 2]] = &[[0, 2], [2, 1]];

#[test]
#[ignore = "daily"]
fn a_pack_whose_clouds_vanilla_reads_past_fails_the_load() {
    if run_fresh_cases(
        NAME,
        CASE,
        &[
            "reload",
            "startup",
            "shader_reload",
            "shader_startup",
            "startup_rollback_pending",
            "filtered_reload",
        ],
    ) {
        return;
    }
    write_pack("inner", INNER);
    write_pack("past", PAST);
    write_pack_with(
        "oit_broken",
        INNER,
        &[(
            "assets/minecraft/shaders/core/clouds.fsh",
            OIT_ONLY_BROKEN_FSH,
        )],
    );
    match std::env::var(CASE).unwrap().as_str() {
        "reload" => a_reload_fails_and_drops_the_selection("file/past"),
        "filtered_reload" => {
            write_pack_with(
                "filtered",
                INNER,
                &[(
                    "pack.mcmeta",
                    r#"{
                    "pack":{"description":"filter clouds","min_format":[97,0],"max_format":[97,1]},
                    "filter":{"block":[{"namespace":"minecraft","path":"shaders/core/clouds[.]fsh"}]}
                }"#,
                )],
            );
            a_reload_fails_and_drops_the_selection("file/filtered");
        }
        "startup_rollback_pending" => startup_rollback_waits_for_world_resources(),
        "shader_reload" => a_reload_fails_and_drops_the_selection("file/oit_broken"),
        "shader_startup" => a_startup_load_fails_back_to_the_games_resources(
            "file/oit_broken",
            "failed to load required shader program minecraft:pipeline/oit_transmittance_clouds",
        ),
        _ => a_startup_load_fails_back_to_the_games_resources(
            "file/past",
            "assets/minecraft/textures/environment/clouds.png: (5, 2) outside of image bounds (4, 6)",
        ),
    }
}

/// A device refusal during the startup handoff arrives before the world resource
/// loader has necessarily joined. Its rollback clears the saved selection at once,
/// but stages the replacement only after those resources are ready.
fn startup_rollback_waits_for_world_resources() {
    use {sunrise_gameplay_common::game::Progress, sunrise_gameplay_common::game::WorldRestLoader};
    with_client_game(|game| {
        let (release, wait) = std::sync::mpsc::channel();
        game.rest_loader = Some(WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(move || wait.recv().map_err(|e| e.to_string())),
        });
        game.client.settings.resource_packs = vec!["file/inner".into()];
        game.selection_reload_failed("device refused the clouds pipeline");
        assert!(game.client.settings.resource_packs.is_empty());
        assert!(
            game.client
                .toast_manager
                .system_mut(
                    sunrise_client_gameplay::game::toast_manager::SystemToastId::PackLoadFailure
                )
                .is_some()
        );
        assert!(game.client.pending_pack_reload);
        assert!(game.client.pack_reload.0.is_none());
        game.resume_world_entry().unwrap();
        assert!(game.client.pack_reload.0.is_none());
        release.send(()).unwrap();
        game.rest_loader.take().unwrap().finish().unwrap();
        game.resume_world_entry().unwrap();
        assert!(!game.client.pending_pack_reload);
        assert!(game.client.pack_reload.0.is_some());
        while game.client.pack_reload.0.is_some() {
            game.drive_pack_reload();
            std::thread::yield_now();
        }
        assert!(game.client.gui.atlas.sky_shader.is_none());
    });
}

/// The accepted pack loads, its sky shader and clouds in use. Selecting the `refused` one fails
/// the reload (Improved Transparency off, as it is by default): the selection is cleared and
/// saved, the game reloads its own resources (no pack sky, vanilla's clouds, no pack clouds
/// shader), and the load-failure toast shows, as vanilla's reload does.
pub(super) fn a_reload_fails_and_drops_the_selection(refused: &str) {
    with_client_game(|game| {
        let vanilla_clouds = game.client.client_pack.clouds.clone();
        game.client.settings.resource_packs = vec!["file/inner".into()];
        reload(game);
        assert_eq!(game.client.settings.resource_packs, ["file/inner"]);
        assert!(game.client.gui.atlas.sky_shader.is_some());
        assert_eq!(
            (
                game.client.client_pack.clouds.0,
                game.client.client_pack.clouds.1
            ),
            (4, 6)
        );
        assert!(!game.client.settings.improved_transparency);
        game.client.settings.resource_packs = vec!["file/inner".into(), refused.into()];
        reload(game);
        assert!(game.client.settings.resource_packs.is_empty());
        assert!(game.client.gui.atlas.clouds_shader.is_none());
        assert!(
            game.client.gui.atlas.sky_shader.is_none(),
            "the pack's sky went too"
        );
        assert_eq!(game.client.client_pack.clouds, vanilla_clouds);
        assert!(
            game.client
                .toast_manager
                .system_mut(
                    sunrise_client_gameplay::game::toast_manager::SystemToastId::PackLoadFailure
                )
                .is_some(),
            "resourcePack.load_fail"
        );
    });
}

/// A `refused` pack selected at startup: `clearResourcePacksOnError` saves the cleared
/// selection before loading the game's own resources and reporting the original `error`.
fn a_startup_load_fails_back_to_the_games_resources(refused: &str, error: &str) {
    use crate::game::*;
    #[cfg(test)]
    use sunrise_gameplay_common::game::*;
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let mut settings = Settings {
        resource_packs: vec![refused.to_owned()],
        ..Settings::default()
    };
    let sources = mods::ModsState::default();
    let roots = sunrise_client_gameplay::game::client_side::packs::startup_root(
        &root,
        &settings.resource_packs,
        &[],
        &sources,
    )
    .unwrap();
    let loaded = load_resources_sync(
        roots,
        Vec::new(),
        StartupLoadContext {
            vanilla_root: &root,
            launch_world: None,
            started: std::time::Instant::now(),
        },
        &mut settings,
        execution_role::ExecutionRole::Client,
        &Progress::new(),
        &sources,
    )
    .unwrap();
    assert!(settings.resource_packs.is_empty());
    let pack_error = loaded.pack_error.unwrap();
    assert!(pack_error.starts_with(error), "{pack_error}");
    let gui = loaded.gui.unwrap();
    assert!(gui.atlas.sky_shader.is_none());
    assert!(gui.atlas.clouds_shader.is_none());
    let vanilla = sunrise_assets::resource_source::ResourceSource::directory(root);
    assert_eq!(
        *loaded.client_pack.unwrap().clouds,
        sunrise_client::clouds::load(&vanilla).unwrap()
    );
}

#[test]
#[ignore = "daily"]
fn content_entity_stays_registered_after_startup_pack_rollback() {
    use crate::game::*;

    if run_fresh_cases(
        "cut_layout::game::client_side::packs::clouds_reload_tests::content_entity_stays_registered_after_startup_pack_rollback",
        "SUNRISE_CONTENT_ENTITY_PACK_ROLLBACK_CASE",
        &["startup"],
    ) {
        return;
    }
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    const ENTITY: &str = "rollback_entity:moss_zombie";
    let game_dir = std::env::current_dir().unwrap();
    let mod_dir = game_dir.join(sunrise_mod_package::mod_list::DIR);
    std::fs::create_dir(&mod_dir).unwrap();
    let package = sunrise_mod_package::Package::write_files(
        "id = 'rollback_entity'\nname = 'Rollback entity'\nversion = '1'\napi = '0.2'\ncapabilities = ['registries']\n",
        &[(
            "data/rollback_entity/sunrise/entity_type/moss_zombie.json",
            br#"{"template":"minecraft:zombie"}"#,
        )],
    )
    .unwrap();
    std::fs::write(mod_dir.join("rollback_entity.smod"), package).unwrap();
    let mut sources = mods::ModsState::default();
    sources.select(&game_dir, &["rollback_entity".to_owned()].into());
    write_pack("past", PAST);

    let vanilla = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let load = |selected: &[String]| {
        let roots = sunrise_client_gameplay::game::client_side::packs::startup_root(
            &vanilla,
            selected,
            &[],
            &sources,
        )
        .unwrap();
        load_resources_sync(
            roots,
            Vec::new(),
            StartupLoadContext {
                vanilla_root: &vanilla,
                launch_world: None,
                started: std::time::Instant::now(),
            },
            &mut Settings::default(),
            execution_role::ExecutionRole::Client,
            &Progress::new(),
            &sources,
        )
        .expect("selecting or removing client packs must not change the entity registry")
    };
    let rolled_back = load(&["file/past".into()]);
    assert!(
        rolled_back
            .pack_error
            .as_deref()
            .unwrap()
            .contains("clouds.png")
    );
    let first_key = mod_api::__private::resolved_entity_type_key(ENTITY);
    let first_spawn_type = rolled_back
        .resources
        .rest()
        .spawner_types
        .contains_key(ENTITY);
    drop(rolled_back);

    std::fs::remove_dir_all(
        sunrise_client_gameplay::game::client_side::packs::packs_dir().join("past"),
    )
    .unwrap();
    let reloaded = load(&[]);
    assert!(reloaded.pack_error.is_none());
    let key = mod_api::__private::resolved_entity_type_key(ENTITY).expect("content entity API key");
    assert_eq!(first_key, Some(key));
    assert!(
        first_spawn_type,
        "spawners must see the registered entity after rollback"
    );
    assert!(reloaded.resources.rest().spawner_types.contains_key(ENTITY));
}

#[test]
#[ignore = "daily"]
fn startup_saves_the_cleared_selection_even_when_a_required_pack_fails() {
    use std::{path::PathBuf, time::Instant};
    use sunrise_assets::resource_source::ResourceRoot;
    use sunrise_client::{
        clouds,
        options_file::{OptionsFile, V},
        packs::{self, Source},
    };
    use {
        crate::settings::Settings,
        sunrise_client_gameplay::game::client_side::resource_load_client::StartupLoadContext,
        sunrise_client_gameplay::game::client_side::resource_load_client::load_resources_sync,
        sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
        sunrise_gameplay_common::game::mods,
    };

    const CASE: &str = "SUNRISE_FAILED_STARTUP_PACK_RECOVERY_CASE";
    const REQUIRED_FAILURE: &str =
        "assets/minecraft/textures/environment/clouds.png: (5, 2) outside of image bounds (4, 6)";
    if run_fresh_cases(
        "cut_layout::game::client_side::packs::clouds_reload_tests::startup_saves_the_cleared_selection_even_when_a_required_pack_fails",
        CASE,
        &["open", "load"],
    ) {
        return;
    }

    let fails_opening = std::env::var(CASE).unwrap() == "open";
    let vanilla = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let dir = sunrise_client_gameplay::game::client_side::packs::packs_dir();
    write_pack("required", PAST);
    write_pack("optional", PAST);
    std::fs::write(
        dir.join("optional").join(clouds::TEXTURE),
        png(3, 5, &[[0, 2]]),
    )
    .unwrap();
    let mut all = packs::folder(&dir, packs::RESOURCE_FORMAT);
    let required = all.iter_mut().find(|p| p.id == "file/required").unwrap();
    let mut info = sunrise_client_gameplay::game::client_side::packs::info(required);
    info.required = true;
    info.position = packs::Position::Bottom;
    required.selection = Some(info);
    if fails_opening {
        // The selected ZIP disappeared between discovery and opening its layer.
        all.iter_mut()
            .find(|p| p.id == "file/optional")
            .unwrap()
            .source = Source::Zip(dir.join("removed.zip"));
    }
    let mut settings = Settings {
        resource_packs: vec!["file/optional".into()],
        ..Settings::default()
    };
    settings.file.set(
        "incompatibleResourcePacks",
        V::List(settings.resource_packs.clone()),
    );
    let path = sunrise_gameplay_common::game::options_path();
    let saved = || OptionsFile::parse(&std::fs::read_to_string(&path).unwrap());
    settings.save(&path);
    assert_eq!(saved().list("resourcePacks"), ["file/optional"]);
    assert_eq!(saved().list("incompatibleResourcePacks"), ["file/optional"]);
    let roots = sunrise_client_gameplay::game::client_side::packs::PackRoots::for_selection(
        ResourceRoot::directory(vanilla.clone()),
        &all,
        &settings.resource_packs,
        &[],
    );
    if fails_opening {
        assert_eq!(
            roots.selected.as_ref().unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    } else {
        let source = roots.selected.as_ref().unwrap().source();
        assert_eq!(
            clouds::load(source).unwrap_err().to_string(),
            "assets/minecraft/textures/environment/clouds.png: (4, 2) outside of image bounds (3, 5)",
        );
    }
    let error = load_resources_sync(
        roots,
        Vec::new(),
        StartupLoadContext {
            vanilla_root: &vanilla,
            launch_world: None,
            started: Instant::now(),
        },
        &mut settings,
        ExecutionRole::Client,
        &Progress::new(),
        &mods::ModsState::default(),
    )
    .err()
    .expect("the required pack still fails after the optional selection is cleared");
    assert_eq!(error.to_string(), REQUIRED_FAILURE);
    assert!(settings.resource_packs.is_empty());
    assert!(saved().list("resourcePacks").is_empty());
    assert!(saved().list("incompatibleResourcePacks").is_empty());
}
