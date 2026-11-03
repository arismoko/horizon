//! The launcher's generated executable for trait mods.
use super::Bundle;
use std::fs;
use std::path::Path;

pub(super) fn read(path: &Path) -> Result<toml::Table, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.parse().map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, value: &toml::Table) -> Result<(), String> {
    let text = toml::to_string(value).map_err(|e| format!("{}: {e}", path.display()))?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) const PACKAGE: &str = "horizon-native-game";
/// Where the generated executable's package is staged, in the workspace.
const GAME: &str = "crates/native-game";

fn path_dep(path: &str) -> toml::Value {
    toml::Value::Table(toml::Table::from_iter([("path".into(), path.into())]))
}

const MAIN_TEMPLATE: &str = "crates/horizon-client/src/native_main.rs.template";

fn main_rs(template: &str, ids: &[&str]) -> String {
    let entries = ids
        .iter()
        .enumerate()
        .map(|(i, id)| format!("        {id:?} => mod_{i}::hooks,\n"))
        .collect::<String>();
    template.replace("/* MOD_ENTRIES */", &entries)
}

/// Add API packages and the generated executable to the fresh staged workspace.
pub(super) fn stage(
    stage: &Path,
    bundles: &[Bundle],
    declared: &[Vec<super::dependencies::Dependency>],
    resolution: &super::dependencies::Resolution,
) -> Result<bool, String> {
    let mods: Vec<_> = resolution.order.iter().map(|&i| (i, &bundles[i])).collect();
    if mods.is_empty() {
        return Ok(false);
    }
    let template = fs::read_to_string(stage.join(MAIN_TEMPLATE)).map_err(|e| {
        format!("{}: {e}", crate::i18n::tr("This game source has no readable native entry template. Update Horizon and rebuild the mods against this game."))
    })?;
    let mut packages = std::collections::HashMap::new();
    for (_, bundle) in &mods {
        let path = bundle.dir.join("Cargo.toml");
        let manifest = read(&path)?;
        let name = manifest
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("{}: missing package name", path.display()))?;
        packages.insert(bundle.manifest.id.as_str(), name.to_owned());
    }
    let mut deps = toml::Table::new();
    for name in ["horizon-client", "horizon-threads", "horizon-mods"] {
        deps.insert(name.into(), path_dep(&format!("../{name}")));
    }
    // horizon-client's defaults (sound, controllers) are this package's, so a server build's
    // `--no-default-features` turns them off here too.
    if let Some(client) = deps
        .get_mut("horizon-client")
        .and_then(toml::Value::as_table_mut)
    {
        client.insert("default-features".into(), false.into());
    }
    let mut members = Vec::new();
    for (i, (index, bundle)) in mods.iter().enumerate() {
        let relative = format!("native-api/{}", bundle.manifest.id);
        let dest = stage.join(&relative);
        super::copy_bundle(bundle, &dest)?;
        let path = dest.join("Cargo.toml");
        let mut manifest = read(&path)?;
        let package = packages[bundle.manifest.id.as_str()].clone();
        let dependencies = manifest
            .get_mut("dependencies")
            .and_then(toml::Value::as_table_mut)
            .ok_or_else(|| format!("{}: missing horizon-client dependency", path.display()))?;
        let dependency = dependencies
            .iter_mut()
            .find(|(key, value)| {
                *key == "horizon-client"
                    || value.get("package").and_then(toml::Value::as_str) == Some("horizon-client")
            })
            .ok_or_else(|| format!("{}: missing horizon-client dependency", path.display()))?
            .1;
        let mut rewritten = path_dep("../../crates/horizon-client");
        if let (Some(old), Some(new)) = (dependency.as_table(), rewritten.as_table_mut()) {
            // A renamed dependency also needs its package name.
            for key in ["package", "features", "default-features"] {
                if let Some(value) = old.get(key) {
                    new.insert(key.into(), value.clone());
                }
            }
        }
        // The game's features are the generated package's to choose (a server build turns its
        // defaults off), so no mod turns them back on.
        if let Some(new) = rewritten.as_table_mut() {
            new.insert("default-features".into(), false.into());
        }
        *dependency = rewritten;
        let targets: Vec<_> = declared[*index]
            .iter()
            .map(|dependency| super::dependencies::Package {
                dependency,
                package: packages.get(dependency.id.as_str()).map(String::as_str),
            })
            .collect();
        let enabled = super::dependencies::rewrite(&bundle.manifest.id, &mut manifest, &targets)
            .map_err(|e| e.to_string())?;
        write(&path, &manifest)?;
        let mut dep = toml::Table::new();
        dep.insert("path".into(), format!("../../{relative}").into());
        dep.insert("package".into(), package.into());
        if !enabled.is_empty() {
            dep.insert(
                "features".into(),
                toml::Value::Array(enabled.into_iter().map(toml::Value::String).collect()),
            );
        }
        deps.insert(format!("mod_{i}"), toml::Value::Table(dep));
        members.push(relative);
    }
    // Beside horizon-client, so its build script (the Windows stack size and icon, for the
    // `horizon-client` binary this package owns now) finds `packaging/` where it looks.
    let game = stage.join(GAME);
    fs::create_dir_all(game.join("src")).map_err(|e| e.to_string())?;
    let mut manifest = toml::Table::new();
    manifest.insert(
        "package".into(),
        toml::Value::Table(toml::Table::from_iter([
            ("name".into(), PACKAGE.into()),
            ("version".into(), "0.1.0".into()),
            ("edition".into(), "2024".into()),
            ("publish".into(), false.into()),
            ("build".into(), "../horizon-client/build.rs".into()),
        ])),
    );
    // This sibling package uses the client's build.rs, so its build dependencies apply too;
    // relative paths still point to the same crates from crates/native-game.
    let client = read(&stage.join("crates/horizon-client/Cargo.toml"))?;
    if let Some(dependencies) = client.get("build-dependencies") {
        manifest.insert("build-dependencies".into(), dependencies.clone());
    }
    let windows = toml::Table::from_iter([(
        "build-dependencies".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "winresource".into(),
            "0.1".into(),
        )])),
    )]);
    manifest.insert(
        "target".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "cfg(windows)".into(),
            toml::Value::Table(windows),
        )])),
    );
    manifest.insert(
        "bin".into(),
        toml::Value::Array(vec![toml::Value::Table(toml::Table::from_iter([
            ("name".into(), "horizon-client".into()),
            ("path".into(), "src/main.rs".into()),
        ]))]),
    );
    manifest.insert("dependencies".into(), toml::Value::Table(deps));
    manifest.insert(
        "features".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "default".into(),
            toml::Value::Array(vec!["horizon-client/default".into()]),
        )])),
    );
    write(&game.join("Cargo.toml"), &manifest)?;
    let ids: Vec<_> = mods
        .iter()
        .map(|(_, bundle)| bundle.manifest.id.as_str())
        .collect();
    fs::write(game.join("src/main.rs"), main_rs(&template, &ids)).map_err(|e| e.to_string())?;
    let root = stage.join("Cargo.toml");
    let mut workspace = read(&root)?;
    let list = workspace
        .get_mut("workspace")
        .and_then(|w| w.get_mut("members"))
        .and_then(toml::Value::as_array_mut)
        .ok_or("workspace has no members")?;
    members.push(GAME.into());
    list.extend(members.into_iter().map(toml::Value::String));
    write(&root, &workspace)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let name = std::thread::current().name().unwrap().replace(':', "-");
        let dir = super::super::tests::dir(&name);
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='example-mod'\nversion='9.0.0'\nedition='2024'\n[dependencies]\nhorizon-client='0.1'\n").unwrap();
        dir
    }

    #[test]
    fn generates_only_tool_contract_tables_without_mutating_package() {
        let dir = fixture();
        let path = dir.join(horizon_mods::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap()
            + "[[native.injection]]\nid='test'\ntarget='horizon_client::game::tick'\nkind='HEAD'\nhelper='src/lib.rs'\n";
        fs::write(&path, &text).unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(bundle.needs_mixins());
        super::super::package::tool_manifest(&bundle).unwrap();
        let value = read(&dir.join(super::super::MANIFEST)).unwrap();
        assert!(!value.contains_key("api"));
        assert!(!value.contains_key("native"));
        assert!(value.contains_key("injection"));
        assert_eq!(value["mod"]["id"].as_str(), Some("example"));
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_main_keeps_mod_order() {
        let template = include_str!("../../../crates/horizon-client/src/native_main.rs.template");
        let ids = ["base", "extra"];
        for count in 0..=2 {
            let text = main_rs(template, &ids[..count]);
            let facts: Vec<_> = text
                .lines()
                .filter(|line| line.contains(" => mod_"))
                .collect();
            assert_eq!(
                facts,
                &[
                    "        \"base\" => mod_0::hooks,",
                    "        \"extra\" => mod_1::hooks,",
                ][..count]
            );
        }
    }

    #[test]
    fn stages_api_without_a_dependency_cycle() {
        let dir = fixture();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        let stage_dir = dir.with_extension("workspace");
        let _ = fs::remove_dir_all(&stage_dir);
        fs::create_dir_all(stage_dir.join("crates/horizon-client/src")).unwrap();
        fs::write(stage_dir.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(
            stage_dir.join("crates/horizon-client/Cargo.toml"),
            "[package]\nname = 'horizon-client'\nversion = '0.1.0'\n",
        )
        .unwrap();
        let resolution = super::super::dependencies::Resolution { order: vec![0] };
        let err = stage(
            &stage_dir,
            std::slice::from_ref(&bundle),
            &[vec![]],
            &resolution,
        )
        .unwrap_err();
        assert!(err.contains("no readable native entry template"), "{err}");
        fs::create_dir_all(stage_dir.join("crates/horizon-client/src")).unwrap();
        fs::write(
            stage_dir.join(MAIN_TEMPLATE),
            include_str!("../../../crates/horizon-client/src/native_main.rs.template"),
        )
        .unwrap();
        assert!(stage(&stage_dir, &[bundle], &[vec![]], &resolution).unwrap());
        let root = read(&stage_dir.join("Cargo.toml")).unwrap();
        let members = root["workspace"]["members"].as_array().unwrap();
        assert!(members
            .iter()
            .any(|v| v.as_str() == Some("native-api/example")));
        assert!(members.iter().any(|v| v.as_str() == Some(GAME)));
        let engine = read(&stage_dir.join("native-api/example/Cargo.toml")).unwrap();
        assert_eq!(
            engine["dependencies"]["horizon-client"]["path"].as_str(),
            Some("../../crates/horizon-client")
        );
        let game = read(&stage_dir.join(GAME).join("Cargo.toml")).unwrap();
        assert_eq!(
            game["dependencies"]["mod_0"]["package"].as_str(),
            Some("example-mod")
        );
        assert_eq!(
            game["dependencies"]["mod_0"]["path"].as_str(),
            Some("../../native-api/example")
        );
        assert_eq!(
            game["package"]["build"].as_str(),
            Some("../horizon-client/build.rs")
        );
        assert!(stage_dir
            .join(GAME)
            .join(
                game["dependencies"]["horizon-client"]["path"]
                    .as_str()
                    .unwrap()
            )
            .ends_with("crates/native-game/../horizon-client"));
        // The game's default features are the generated package's alone, so a server build's
        // `--no-default-features` reaches horizon-client.
        assert_eq!(
            engine["dependencies"]["horizon-client"]["default-features"].as_bool(),
            Some(false)
        );
        assert_eq!(
            game["dependencies"]["horizon-client"]["default-features"].as_bool(),
            Some(false)
        );
        assert_eq!(
            game["features"]["default"][0].as_str(),
            Some("horizon-client/default")
        );
        let main = || fs::read_to_string(stage_dir.join(GAME).join("src/main.rs")).unwrap();
        assert!(main().contains(r#""example" => mod_0::hooks,"#));
        fs::write(stage_dir.join("crates/horizon-client/Cargo.toml"), "[package]\nname = 'horizon-client'\nversion = '0.1.0'\n[build-dependencies]\nhorizon-mod-api-gen = { path = '../horizon-mod-api-gen' }\n").unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        stage(&stage_dir, &[bundle], &[vec![]], &resolution).unwrap();
        assert!(main().contains(r#""example" => mod_0::hooks,"#));
        let game = read(&stage_dir.join(GAME).join("Cargo.toml")).unwrap();
        assert_eq!(
            game["build-dependencies"]["horizon-mod-api-gen"]["path"].as_str(),
            Some("../horizon-mod-api-gen")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn named_source_without_an_entry_template_is_refused_before_staging() {
        let dir = fixture();
        let stage_dir = dir.with_extension("workspace");
        let _ = fs::remove_dir_all(&stage_dir);
        let source = stage_dir.join("crates/horizon-client/src");
        fs::create_dir_all(&source).unwrap();
        let resolution = super::super::dependencies::Resolution { order: vec![0] };
        for text in ["", "pub struct ModEntry { id: &'static str }"] {
            fs::write(source.join("mod_api.rs"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            let error = stage(&stage_dir, &[bundle], &[vec![]], &resolution).unwrap_err();
            assert!(error.contains("Update Horizon and rebuild"), "{error}");
            assert!(!stage_dir.join(GAME).exists());
            assert!(!stage_dir.join("native-api").exists());
        }
        assert!(!stage(
            &stage_dir,
            &[],
            &[],
            &super::super::dependencies::Resolution { order: vec![] },
        )
        .unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn api_only_skips_mixins() {
        let dir = fixture();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(!bundle.needs_mixins());
        fs::remove_dir_all(dir).unwrap();
    }
}
