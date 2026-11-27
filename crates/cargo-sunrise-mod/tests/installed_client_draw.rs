#[path = "../src/dev/work_dir.rs"]
mod work_dir;

use std::{fs, path::Path, process::Command};

#[test]
#[ignore = "daily"]
fn installed_lower_bends_and_routes_a_client_model() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(target).unwrap();
    let scratch = tempfile::tempdir_in(target).unwrap();
    let root = scratch.path().canonicalize().unwrap();
    let head = Command::new("git")
        .current_dir(&repo)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success());
    let commit = std::str::from_utf8(&head.stdout).unwrap().trim();
    let project = root.join("lower");
    assert!(
        Command::new("cp")
            .arg("-R")
            .arg(repo.join("tools/ci/fn-replace-check/lower"))
            .arg(&project)
            .status()
            .unwrap()
            .success()
    );
    let cache = root.join("cache");
    // On the carry head GitHub may not have the commit yet. Seed the native builder's normal
    // source cache with the exact tracked revision, as fn_replace_check.sh does.
    let source = work_dir::work_dir(&cache, "fnr_lower", &dunce::canonicalize(&project).unwrap())
        .join("source")
        .join(commit);
    fs::create_dir_all(&source).unwrap();
    let archive = root.join("source.tar");
    assert!(
        Command::new("git")
            .current_dir(&repo)
            .args(["archive", "--format=tar", "--output"])
            .arg(&archive)
            .arg(commit)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("tar")
            .current_dir(&source)
            .arg("-xf")
            .arg(&archive)
            .status()
            .unwrap()
            .success()
    );
    fs::remove_file(archive).unwrap();
    fs::write(source.join(".horizon-revision"), commit).unwrap();
    let manifest = project.join("sunrise-mod.toml");
    let mixins = sunrise_mod_package::mixins_tool::REV;
    let mut content = fs::read_to_string(&manifest)
        .unwrap()
        .replace("sunrise = \"HEAD\"", &format!("sunrise = \"{commit}\""))
        .replace("mixins = \"MIXINS\"", &format!("mixins = \"{mixins}\""));
    // Keep the dedicated fixture unchanged. This extra row starts the client-only draw after
    // the loader has installed the normal, unchanged lower rows.
    content.push_str(
        r#"
[[native.mixin]]
id = "client_draw"
target = "sunrise_gameplay_common::game::preflight_resources"
kind = "HEAD"
helper = "src/mixins/client_draw.rs"
"#,
    );
    fs::write(manifest, content).unwrap();

    // Startup reaches native loading without Mojang's resources, a world or a display.
    let resources = root.join("resources");
    fs::create_dir(&resources).unwrap();
    for name in ["sunrise-resources.json", "registries.json"] {
        fs::write(resources.join(name), "{}").unwrap();
    }
    fs::write(
        resources.join("entity-types.json"),
        r#"{"minecraft:wolf":{}}"#,
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .current_dir(&project)
        .env("SUNRISE_MOD_CACHE", &cache)
        .env("SUNRISE_FNR_LOWER_DRAW", &resources)
        .env("SUNRISE_HOME", root.join("home"))
        .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .args(["run", "--resources"])
        .arg(&resources)
        .args(["--", "--offscreen"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    let stderr = String::from_utf8_lossy(&child.stderr);
    assert!(child.status.success(), "{stdout}\n{stderr}");
    for marker in [
        "RETURN mobs::anim::setup",
        "HEAD mobs::render::compile_part",
        "RETURN mobs::render::takes_part",
        "installed client draw passed",
    ] {
        let expected = format!("fnr: lower {marker}");
        assert!(
            stderr.lines().any(|line| line == expected),
            "{stdout}\n{stderr}"
        );
    }
}
