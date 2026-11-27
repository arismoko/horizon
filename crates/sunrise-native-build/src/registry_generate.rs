//! Explicit, index-only generation of the registry lock consumed by native staging.
use super::{Bundle, api, registry_lock::RegistryLock};
use crate::rust::Toolchain;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

/// For `native-lock`, emit only inert manifests; no build scripts, source or Cargo config.
fn stage(project: &api::Project<'_>, output: &Path) -> Result<(), String> {
    let write = |relative: &Path, manifest: &mut toml::Table| -> Result<(), String> {
        manifest.remove("bin");
        manifest.remove("example");
        manifest.remove("test");
        manifest.remove("bench");
        manifest.remove("lib");
        let package = manifest
            .get_mut("package")
            .and_then(toml::Value::as_table_mut)
            .ok_or("Cargo.toml: missing package")?;
        package.insert("build".into(), false.into());
        for key in ["autobins", "autoexamples", "autotests", "autobenches"] {
            package.insert(key.into(), false.into());
        }
        manifest.insert("lib".into(), toml::toml! { path = "inert.rs" }.into());
        if relative.as_os_str().is_empty() {
            manifest.insert("workspace".into(), toml::Table::new().into());
        }
        let dest = output.join(relative);
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        fs::write(dest.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
        fs::write(dest.join("inert.rs"), "").map_err(|e| e.to_string())
    };
    let (mut root, _) = super::crates::rewrite_closure(project, &project.bundle.dir, write)?;
    write(Path::new(""), &mut root)
}

/// The mod resolver's seed from the pinned game's `Cargo.lock`. The kept file is the original
/// game lock, not a projected lock. A kept file that no longer reads as a seed is downloaded again.
pub fn game_lock(revision: &str) -> Result<RegistryLock, String> {
    game_lock_at(revision, &super::root(), crate::host::get)
}

fn game_lock_at(
    revision: &str,
    root: &Path,
    get: impl FnOnce(&str) -> Result<Box<dyn Read + Send>, String>,
) -> Result<RegistryLock, String> {
    let kept = root.join("game-lock").join(format!("{revision}.lock"));
    if let Ok(file) = fs::File::open(&kept)
        && let Ok(text) = read_game_lock(file)
        && let Ok(seed) = RegistryLock::parse_game_seed(&text)
    {
        crate::host::mark_used(&kept)?;
        return Ok(seed);
    }
    let url = format!(
        "{}/{revision}/Cargo.lock",
        sunrise_addresses::GAME_SOURCE_FILES
    );
    let unavailable = |error: String| {
        crate::host::trf(
            "The mod's lock starts from the Cargo.lock of its game build, {revision}, which could not be downloaded: {error}. To lock without the network, put that file at {path}.",
            &[
                ("revision", revision),
                ("error", &error),
                ("path", &kept.display().to_string()),
            ],
        )
    };
    let text = read_game_lock(get(&url).map_err(unavailable)?).map_err(unavailable)?;
    let seed = RegistryLock::parse_game_seed(&text).map_err(unavailable)?;
    let folder = kept.parent().expect("a file in the build's folder");
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let mut pending = tempfile::NamedTempFile::new_in(folder).map_err(|e| e.to_string())?;
    pending
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    pending.persist(&kept).map_err(|e| e.to_string())?;
    crate::host::mark_used(&kept)?;
    Ok(seed)
}

/// More than any game lock holds; a larger input is not one.
const GAME_LOCK_LIMIT: u64 = 16 << 20;

fn read_game_lock(input: impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    // A truncated lock can still parse. Read one extra byte to detect overflow first.
    input
        .take(GAME_LOCK_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > GAME_LOCK_LIMIT {
        return Err("the answer is too large to be a lock".into());
    }
    String::from_utf8(bytes).map_err(|e| e.utf8_error().to_string())
}

/// The mod's registry pins resolved with the toolchain `rust` gives, once the mod's inputs passed.
/// Resolution starts from `game`, the `Cargo.lock` of the mod's game build: a package the game
/// also locks keeps the game's version, so mods made for one build agree on those packages, and
/// only the mod's own packages take the newest version that fits. `game` is asked for the lock of
/// the build the mod pins.
pub fn generate<T: Toolchain>(
    rust: impl FnOnce() -> Result<T, String>,
    game: impl FnOnce(&str) -> Result<RegistryLock, String>,
    bundle: &Bundle,
    update: bool,
) -> Result<(), String> {
    let native = bundle.manifest.native.as_ref().ok_or_else(|| {
        crate::host::trf(
            "{path} is not a native mod.",
            &[("path", &bundle.dir.display().to_string())],
        )
    })?;
    // Project owns the original API declaration and source-policy boundary.
    let project = api::Project::read(bundle).map_err(|e| e.to_string())?;
    let destination = bundle.dir.join("Cargo.lock");
    if destination.exists() && !update {
        return Err(
            "Cargo.lock already exists; use --update-lock to replace its pins explicitly".into(),
        );
    }
    fs::create_dir_all(super::root()).map_err(|e| e.to_string())?;
    let work = tempfile::tempdir_in(super::root()).map_err(|e| e.to_string())?;
    stage(&project, work.path())?;
    let seed = game(&native.sunrise)?;
    let rust = rust()?;
    let text = super::registry_resolve::resolve(work.path(), &seed, &seed, None, |subcommand| {
        super::cargo(&rust, work.path(), subcommand)
    })
    .map_err(|error| {
        crate::host::trf(
            "{mod}: could not write Cargo.lock.\n{error}",
            &[("mod", &bundle.manifest.id), ("error", &error)],
        )
    })?;
    // Refusals and resolution errors above never mutate the existing lock.
    let mut pending = tempfile::NamedTempFile::new_in(&bundle.dir).map_err(|e| e.to_string())?;
    pending
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    pending.as_file().sync_all().map_err(|e| e.to_string())?;
    pending.persist(destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Bundle {
        super::super::tests::write_bundle(
            root,
            "registry_fixture",
            "compile_error!(\"must never compile\");",
        );
        fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nsunrise-mod-api={ path='.sunrise/sdk/api/crates/sunrise-mod-api' }\nitoa='1'\n").unwrap();
        super::super::bundle_in(root).unwrap().unwrap()
    }
    fn stage_bundle(bundle: &Bundle, output: &Path) -> Result<(), String> {
        let project = api::Project::read(bundle).map_err(|e| e.to_string())?;
        stage(&project, output)
    }

    #[test]
    fn kept_and_downloaded_game_locks_share_the_byte_limit() {
        let root = tempfile::tempdir().unwrap();
        let revision = "a".repeat(40);
        let kept = root
            .path()
            .join("game-lock")
            .join(format!("{revision}.lock"));
        let lock = include_bytes!("../../../tools/fixtures/sdk-git-patches/game.lock");
        game_lock_at(&revision, root.path(), |_| {
            Ok(Box::new(std::io::Cursor::new(lock)))
        })
        .unwrap();
        assert_eq!(fs::read(&kept).unwrap().as_slice(), lock);
        game_lock_at(&revision, root.path(), |_| {
            panic!("valid kept seed is offline")
        })
        .unwrap();

        {
            let mut file = fs::OpenOptions::new().append(true).open(&kept).unwrap();
            // Still valid TOML if read in full or truncated. Only the byte limit rejects it.
            let mut padding = std::io::repeat(b' ').take(GAME_LOCK_LIMIT + 1 - lock.len() as u64);
            std::io::copy(&mut padding, &mut file).unwrap();
        }
        let error = game_lock_at(&revision, root.path(), |_| {
            Ok(Box::new(fs::File::open(&kept).unwrap()))
        })
        .unwrap_err();
        assert!(
            error.contains("the answer is too large to be a lock"),
            "{error}"
        );
        assert!(error.contains("To lock without the network"), "{error}");
        assert_eq!(fs::metadata(&kept).unwrap().len(), GAME_LOCK_LIMIT + 1);
    }

    #[test]
    fn the_sdk_is_removed_only_in_its_path_form_and_sources_are_inert() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        fs::write(input.path().join("build.rs"), "panic!(\"do not run\");").unwrap();
        fs::create_dir_all(input.path().join(".cargo")).unwrap();
        fs::write(
            input.path().join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='local'\n",
        )
        .unwrap();
        let before = fs::read(input.path().join("Cargo.toml")).unwrap();
        stage_bundle(&bundle, output.path()).unwrap();
        assert!(!output.path().join(".cargo").exists());
        let generated = fs::read_to_string(output.path().join("Cargo.toml")).unwrap();
        assert!(!generated.contains("sunrise-mod-api"));
        assert!(generated.contains("itoa"));
        assert!(!output.path().join("build.rs").exists());
        assert!(!output.path().join("src").exists());
        assert_eq!(fs::read(input.path().join("Cargo.toml")).unwrap(), before);
        let original = String::from_utf8(before).unwrap();
        for (form, refusal) in [
            (
                format!(
                    "{{ git='https://github.com/arismoko/horizon', rev='{}' }}",
                    "a".repeat(40)
                ),
                "E_SDK_SOURCE: sunrise-mod-api: the SDK is no longer named by git",
            ),
            (
                "'0.1'".to_owned(),
                "E_SDK_SOURCE: sunrise-mod-api: a version names no source",
            ),
            (
                "{ path='../sunrise/crates/sunrise-mod-api' }".to_owned(),
                "E_SDK_SOURCE: sunrise-mod-api: the path must be \".sunrise/sdk/api/crates/sunrise-mod-api\"",
            ),
        ] {
            fs::write(
                input.path().join("Cargo.toml"),
                original.replace("{ path='.sunrise/sdk/api/crates/sunrise-mod-api' }", &form),
            )
            .unwrap();
            let error = stage_bundle(&bundle, output.path()).unwrap_err();
            assert!(error.contains(refusal), "{error}");
        }
    }
    #[test]
    fn source_refusals_leave_existing_lock_untouched() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        let original = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        fs::write(input.path().join("Cargo.lock"), "original pins").unwrap();
        let unreached = || -> Result<crate::rust::Unpacked, String> {
            panic!("no toolchain before the refusal")
        };
        assert!(
            generate(
                unreached,
                |_| panic!("no game lock before the refusal"),
                &bundle,
                false
            )
            .unwrap_err()
            .contains("--update-lock")
        );
        for (extra, refusal) in [
            (
                "[patch.crates-io]\nitoa={path='outside'}\n",
                "[patch] is not allowed",
            ),
            (
                "[replace]\n'itoa:1.0.18'={path='outside'}\n",
                "[replace] is not allowed",
            ),
            (
                "git_only={git='https://example.invalid/mod',rev='1111111111111111111111111111111111111111'}\n",
                "git_only: git source is refused",
            ),
        ] {
            fs::write(
                input.path().join("Cargo.toml"),
                format!("{original}\n{extra}"),
            )
            .unwrap();
            let error = stage_bundle(&bundle, output.path()).unwrap_err();
            assert!(error.contains(refusal), "{error}");
            assert_eq!(
                fs::read_to_string(input.path().join("Cargo.lock")).unwrap(),
                "original pins"
            );
        }
    }
    #[test]
    fn local_dependencies_are_traversed_but_cannot_escape() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        let original = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        fs::create_dir(input.path().join("local")).unwrap();
        fs::write(
            input.path().join("local/Cargo.toml"),
            "[package]\nname='helper'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(
            input.path().join("Cargo.toml"),
            format!("{original}\nhelper={{path='local'}}\n"),
        )
        .unwrap();
        stage_bundle(&bundle, output.path()).unwrap();
        assert!(output.path().join("local/inert.rs").is_file());
        fs::write(
            input.path().join("Cargo.toml"),
            format!("{original}\nhelper={{path='../outside'}}\n"),
        )
        .unwrap();
        assert!(
            stage_bundle(&bundle, output.path())
                .unwrap_err()
                .contains("outside the mod")
        );
    }

    struct GameSeedFixture {
        input: tempfile::TempDir,
        bundle: Bundle,
        game: String,
        toolchain: crate::rust::Unpacked,
    }

    fn game_seed_fixture() -> GameSeedFixture {
        let input = tempfile::tempdir().unwrap();
        let vendor = input.path().join("vendor");
        let bundle = fixture(input.path());
        for (name, version, dependency) in [
            ("zipish", "1.0.0", "getrandom='0.4'"),
            ("zipish", "1.0.1", "getrandom='=0.4.4'"),
            ("getrandom", "0.4.3", "libc='0.2'"),
            ("getrandom", "0.4.4", "libc='0.2'"),
            ("libc", "0.2.190", ""),
            ("libc", "0.2.191", "[features]\nnew_api=[]"),
            ("libc", "0.3.0", ""),
            ("unused", "1.0.0", ""),
        ] {
            let folder = vendor.join(format!("{name}-{version}"));
            fs::create_dir_all(folder.join("src")).unwrap();
            fs::write(folder.join("src/lib.rs"), "").unwrap();
            fs::write(
                folder.join("Cargo.toml"),
                format!(
                    "[package]\nname='{name}'\nversion='{version}'\n[dependencies]\n{dependency}\n"
                ),
            )
            .unwrap();
            fs::write(
                folder.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
            )
            .unwrap();
        }
        fs::write(
            input.path().join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nsunrise-mod-api={ path='.sunrise/sdk/api/crates/sunrise-mod-api' }\nzipish='1'\nother-libc={package='libc',version='0.3'}\n",
        )
        .unwrap();
        let package = |name: &str, version: &str, dependencies: &str| {
            format!(
                "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{}\"\ndependencies = [{dependencies}]\n",
                "a".repeat(64)
            )
        };
        // The game's own crate and a package only the game uses stay out of the mod's lock.
        let game = format!(
            "version = 4\n[[package]]\nname = \"game\"\nversion = \"1.0.0\"\ndependencies = [\"getrandom\", \"unused\"]\n{}{}{}",
            package("getrandom", "0.4.3", "\"libc\""),
            package("libc", "0.2.190", ""),
            package("unused", "1.0.0", "")
        );
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap()
            .stdout;
        let host = std::process::Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .unwrap()
            .stdout;
        let toolchain = crate::rust::Unpacked {
            dir: String::from_utf8(sysroot).unwrap().trim().into(),
            target: String::from_utf8(host).unwrap().trim().into(),
            cargo_home: input.path().join("cargo-home"),
            cargo_config: vec![
                "net.offline=true".into(),
                "source.crates-io.replace-with='fixture'".into(),
                format!("source.fixture.directory='{}'", vendor.display()),
            ],
            c_bin: None,
        };
        GameSeedFixture {
            input,
            bundle,
            game,
            toolchain,
        }
    }

    /// A mod's own package needs packages the game locks: the lock gives the game's version of
    /// each, not the newest, and a build's check takes it. A different semver slot is still free.
    #[test]
    #[ignore = "daily"]
    fn a_mods_lock_starts_from_the_games_versions() {
        let GameSeedFixture {
            input,
            bundle,
            game,
            toolchain,
        } = game_seed_fixture();
        generate(
            || Ok(toolchain),
            |_| RegistryLock::parse_game_seed(&game),
            &bundle,
            false,
        )
        .unwrap();
        let text = fs::read_to_string(input.path().join("Cargo.lock")).unwrap();
        let pins = RegistryLock::parse(&text).unwrap();
        let locked: Vec<_> = pins
            .packages
            .keys()
            .map(|id| format!("{} {}", id.name, id.version))
            .collect();
        assert_eq!(
            locked,
            [
                "getrandom 0.4.3",
                "libc 0.2.190",
                "libc 0.3.0",
                "zipish 1.0.0"
            ],
            "{text}"
        );
        let mut allowed = RegistryLock::parse(&game).unwrap();
        allowed.extend(&pins).unwrap();
        allowed.check_pins(&pins).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_requirement_that_moves_a_game_pin_leaves_the_old_lock_untouched() {
        let GameSeedFixture {
            input,
            bundle,
            game,
            toolchain,
        } = game_seed_fixture();
        let manifest = input.path().join("Cargo.toml");
        let original = fs::read_to_string(&manifest).unwrap();
        let older_parent = original.replace("zipish='1'", "zipish='=1.0.0'");
        let lock = input.path().join("Cargo.lock");
        generate(
            || Ok(toolchain.clone()),
            |_| RegistryLock::parse_game_seed(&game),
            &bundle,
            false,
        )
        .unwrap();
        let old = fs::read(&lock).unwrap();
        for (cargo, expected) in [
            (
                format!(
                    "{older_parent}\n[target.'cfg(unix)'.build-dependencies]\nnewer={{package='libc',version='=0.2.191'}}\n"
                ),
                ["libc", "0.2.190", "=0.2.191", "fixture v0.1.0"],
            ),
            (
                original.replace("zipish='1'", "zipish='=1.0.1'"),
                ["getrandom", "0.4.3", "=0.4.4", "zipish v1.0.1"],
            ),
            (
                format!("{older_parent}\nlibc={{version='0.2',features=['new_api']}}\n"),
                ["libc", "0.2.190", "0.2.191", "`new_api`"],
            ),
        ] {
            fs::write(&manifest, cargo).unwrap();
            let error = generate(
                || Ok(toolchain.clone()),
                |_| RegistryLock::parse_game_seed(&game),
                &bundle,
                true,
            )
            .unwrap_err();
            for detail in expected.into_iter().chain(["registry_fixture"]) {
                assert!(error.contains(detail), "{error}");
            }
            assert_eq!(fs::read(&lock).unwrap(), old);
        }
    }
}
