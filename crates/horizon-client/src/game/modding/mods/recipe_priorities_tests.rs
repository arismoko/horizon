use super::*;
use horizon_client::crafting::ResolvedSources;
use horizon_mod_package::manifest::JarRef;
use horizon_world::{loot::PackOverlay, native_load_context::DataLoadOrigin};
use std::io::Write as _;

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn jar(store: &Path, id: &str, digit: char, files: &[(String, Vec<u8>)]) -> JarRef {
    // As at the real game boundary, the launcher has already addressed this jar.
    let reference = JarRef::new(id.into(), "1".into(), digit.to_string().repeat(128)).unwrap();
    let path = reference.cache_path(store);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut archive = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (path, bytes) in files {
        archive
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
    reference
}

fn package(
    root: &Path,
    id: &str,
    packaged: bool,
    jars: &[JarRef],
    files: &[(String, Vec<u8>)],
) -> PathBuf {
    let mut manifest = format!(
        "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\nside='both'\ncapabilities=['registries']\n"
    );
    if id == "alpha" {
        manifest.push_str("[dependencies]\nzbase='^1'\n");
    }
    for reference in jars {
        manifest.push_str(&format!(
            "[[jar]]\nid='{}'\nversion='1'\nsha512='{}'\n",
            reference.id(),
            reference.sha512(),
        ));
    }
    if packaged {
        let files = files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        let path = root.join(format!("{id}.hmod"));
        write(
            &path,
            horizon_mod_package::Package::write_files(&manifest, &files).unwrap(),
        );
        path
    } else {
        let path = root.join(id);
        write(&path.join("horizon-mod.toml"), manifest);
        for (name, bytes) in files {
            write(&path.join(name), bytes);
        }
        path
    }
}

fn check_layers(kind: &str, identical_path: bool, replace_allowed: bool) {
    let fixture = Fixture::new(&format!(
        "recipe-priorities-{kind}-{identical_path}-{replace_allowed}"
    ));
    let base = fixture.0.join("resources");
    write(&base.join("horizon-resources.json"), "{}");
    let store = fixture.0.join("jars");
    write(&base.join("facts.json"), r#"{"states":[],"defaults":{}}"#);
    write(&base.join("gameplay-facts.json"), r#"{"states":[]}"#);
    write(&base.join("items.json"), "{}");
    std::fs::create_dir_all(base.join("data/minecraft/recipe")).unwrap();
    let lower = br#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"zbase"}],"entries":{"zbase:b":5}}"#.to_vec();
    let higher = format!(
        r#"{{"replace":true,"entries":{{}},"neoforge:conditions":[{{"type":"neoforge:mod_loaded","modid":"{}"}}]}}"#,
        if replace_allowed { "alpha" } else { "not_loaded" },
    ).into_bytes();
    let lower_namespace = if identical_path { "alpha" } else { "zbase" };
    let underlay = br#"{"entries":{"zbase:b":-5}}"#.to_vec();
    let first_jar = jar(
        &store,
        "zfirst",
        'c',
        &[(
            format!("data/{lower_namespace}/recipe_priorities.json"),
            underlay.clone(),
        )],
    );
    let mut lower_files = vec![(
        format!("data/{lower_namespace}/recipe_priorities.json"),
        lower.clone(),
    )];
    for (name, result) in [("a", "stick"), ("b", "diamond")] {
        lower_files.push((
            format!("data/zbase/recipe/{name}.json"),
            format!(
                r#"{{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{{"id":"minecraft:{result}"}}}}"#
            ).into_bytes(),
        ));
    }
    let ordinary_path = "data/alpha/loot_table/ordinary.json";
    let lower_ordinary = br#"{"layer":"lower"}"#.to_vec();
    let higher_ordinary = br#"{"layer":"higher"}"#.to_vec();
    let mut lower_jar_files = vec![(ordinary_path.into(), lower_ordinary)];
    // Shared paths are legal through passive jars without giving two packages
    // ownership of the same namespace. They still use real content admission.
    if identical_path || kind == "jar" {
        lower_jar_files.append(&mut lower_files);
    }
    let lower_jar = jar(&store, "asecond", 'a', &lower_jar_files);
    let higher_files = vec![
        ("data/alpha/recipe_priorities.json".into(), higher.clone()),
        (ordinary_path.into(), higher_ordinary.clone()),
    ];
    let (higher_jars, higher_files) = if kind == "jar" {
        (vec![jar(&store, "higher", 'b', &higher_files)], Vec::new())
    } else {
        (Vec::new(), higher_files)
    };
    let packaged = kind != "directory";
    let zbase = package(
        &fixture.0,
        "zbase",
        packaged,
        &[first_jar, lower_jar],
        &lower_files,
    );
    let alpha = package(&fixture.0, "alpha", packaged, &higher_jars, &higher_files);
    let mut sources = ModsState {
        // Deliberately opposite to dependency order and namespace ordering.
        dev: vec![alpha, zbase],
        ..Default::default()
    };
    sources.select(&fixture.0, &Default::default());
    let overlay = content_overlay(&base, &sources, &store).unwrap();
    assert_eq!(
        overlay
            .mods
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["zbase", "alpha"],
    );
    // Exercise the real world-table caller, not a synthetic DataPack vector.
    let stack = crate::game::functions::pack_stack(
        &fixture.0.join("world"),
        overlay.root.prepared(),
        &horizon_assets::resource_source::ResourceSource::directory(base),
        overlay.root.data_changes(),
        false,
        &[],
        &[],
    );
    let files = crate::game::functions::content_overlay(&stack);
    assert_eq!(
        files
            .iter()
            .find(|(id, _)| id == "recipe_priorities")
            .unwrap()
            .1,
        [underlay, lower, higher],
    );
    assert_eq!(
        overlay.root.data_changes().read(ordinary_path).unwrap(),
        higher_ordinary,
    );
    assert_eq!(
        files
            .iter()
            .find(|(id, _)| id == "loot_table/alpha:ordinary")
            .unwrap()
            .1,
        [higher_ordinary],
    );
    let context = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(
            Default::default(),
            fixture.0.join("settings"),
        )),
        ["zbase", "alpha"],
    );
    let resolved =
        ResolvedSources::load(overlay.root.prepared(), &PackOverlay::new(&files), &context)
            .unwrap();
    assert_eq!(
        resolved
            .recipes
            .iter()
            .map(|recipe| recipe.name.as_str())
            .collect::<Vec<_>>(),
        if replace_allowed {
            ["zbase:a", "zbase:b"]
        } else {
            ["zbase:b", "zbase:a"]
        },
    );
}

#[test]
fn enabled_passive_recipe_priorities_preserve_dependency_layers_and_conditions() {
    const MODE: &str = "HORIZON_PASSIVE_RECIPE_PRIORITIES_TEST";
    if std::env::var_os(MODE).is_none() {
        run_in_child(
            "game::mods::tests::recipe_priorities::enabled_passive_recipe_priorities_preserve_dependency_layers_and_conditions",
            MODE,
        );
        return;
    }
    for kind in ["directory", "package", "jar"] {
        for identical_path in [false, true] {
            for replace_allowed in [false, true] {
                check_layers(kind, identical_path, replace_allowed);
            }
        }
    }
}
