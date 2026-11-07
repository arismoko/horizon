//! Expensive binding coverage over the actual staged game, not a synthetic catalog.
use crate as horizon_mod_api_gen;
use horizon_mod_api_gen::{crossing::Rule, internals_table, internals_view, points, staging};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

fn checked(command: &mut Command) -> std::process::Output {
    let output = command.output().expect("start coverage command");
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
#[ignore = "stages and checks the universal host and wasm32 guest; main-push wasm-mods only"]
fn real_catalog_has_compiling_host_and_guest_bindings() {
    let started = std::time::Instant::now();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let build = root.join("target/internals-catalog-coverage");
    fs::create_dir_all(&build).unwrap();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let wasm_cargo =
        std::env::var_os("HORIZON_COVERAGE_WASM_CARGO").unwrap_or_else(|| cargo.clone());
    let mut guest = Command::new(wasm_cargo);
    if let Some(rustc) = std::env::var_os("HORIZON_COVERAGE_WASM_RUSTC") {
        guest_compiler(
            &mut guest,
            Path::new(&rustc),
            std::env::var_os("LD_LIBRARY_PATH"),
        );
    }
    // Fail on missing target/toolchain before spending time generating the real catalog.
    let probe = build.join("probe");
    fs::create_dir_all(probe.join("src")).unwrap();
    fs::write(
        probe.join("Cargo.toml"),
        "[package]\nname='coverage-probe'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        probe.join("src/lib.rs"),
        "pub fn probe() -> std::vec::Vec<u8> { vec![1] }",
    )
    .unwrap();
    checked(
        guest
            .current_dir(&probe)
            .env("CARGO_TARGET_DIR", build.join("probe-target"))
            .args(["check", "--target", "wasm32-unknown-unknown"]),
    );
    // Reuse the compiler environment, not the probe's arguments.
    let mut check_guest = Command::new(guest.get_program());
    for (key, value) in guest.get_envs() {
        if let Some(value) = value {
            check_guest.env(key, value);
        }
    }
    let files = checked(
        Command::new("git")
            .current_dir(&root)
            .args(["ls-files", "-z"]),
    );
    let paths: Vec<_> = files
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| std::str::from_utf8(p).unwrap())
        .filter(|p| root.join(p).is_file())
        .collect();
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for path in &paths {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        let bytes = fs::read(root.join(path)).unwrap();
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    let fingerprint = format!("{:x}", hash.finalize());
    let stage = build.join("stage");
    let marker = build.join("source.sha256");
    if fs::read_to_string(&marker).ok().as_deref() != Some(&fingerprint) {
        let fresh = build.join("fresh");
        if fresh.exists() {
            fs::remove_dir_all(&fresh).unwrap();
        }
        fs::create_dir_all(&fresh).unwrap();
        for path in paths {
            let dest = fresh.join(path);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::copy(root.join(path), dest).unwrap();
        }
        prepare(&fresh, &build);
        sync_tree(&fresh, &stage);
        fs::remove_dir_all(&fresh).unwrap();
        fs::write(marker, fingerprint).unwrap();
    } else {
        println!("reusing staged catalog for identical tracked sources");
    }
    let report = fs::read_to_string(build.join("summary.md")).unwrap();
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(summary)
            .unwrap()
            .write_all(report.as_bytes())
            .unwrap();
    }
    println!("{report}");
    let target = root.join("target/internals-catalog-check");
    println!("checking staged host");
    checked(
        Command::new(&cargo)
            .current_dir(&stage)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .args([
                "check",
                "--message-format=short",
                "-j4",
                "-p",
                "horizon-client",
                "--lib",
                "--no-default-features",
            ]),
    );
    println!("checking generated wasm32 guest");
    checked(
        check_guest
            .current_dir(&stage)
            .env("CARGO_TARGET_DIR", target.join("wasm"))
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .env(
                "HORIZON_MOD_INTERNALS_MANIFEST",
                build.join("coverage.toml"),
            )
            .env(
                "HORIZON_MOD_INTERNALS_CATALOG",
                stage.join("internals.json"),
            )
            .args([
                "check",
                "--message-format=short",
                "-j4",
                "-p",
                "horizon-mod-internals",
                "--target",
                "wasm32-unknown-unknown",
            ])
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit()),
    );
    println!("real catalog coverage passed in {:?}", started.elapsed());
}

fn prepare(stage: &Path, build: &Path) {
    let catalog = stage.join("internals.json");
    println!("staging real game catalog");
    let mut staged = staging::stage(
        stage,
        &stage.join("points.json"),
        &catalog,
        staging::Mode::Build,
    )
    .unwrap();
    // Shape-only layouts are not mod declarations, but still require a working codec/node on
    // both sides. This coverage catalog exposes them as roots solely to compile every layout.
    for entry in &mut staged.internals.entries {
        entry.shape_only = false;
    }
    let entries = &staged.internals.entries;
    fs::write(&catalog, serde_json::to_vec(entries).unwrap()).unwrap();
    let all: Vec<_> = entries
        .iter()
        .filter(|i| i.cfg.is_empty())
        .map(|i| i.path.clone())
        .collect();
    let game_points = points::game_points(stage).unwrap();
    println!("selecting universal bindings");
    let rule = Rule::new(entries);
    let hashes = entries
        .iter()
        .filter(|i| matches!(i.kind, "struct" | "enum") && i.cfg.is_empty())
        .map(|i| {
            (
                crate::crossing::bare(&i.path),
                (i.path.as_str(), i.hash.as_str()),
            )
        })
        .collect();
    let selected = internals_table::select(
        entries,
        all.iter().map(String::as_str).collect(),
        &game_points,
        &rule,
        &hashes,
    );
    let mut skipped = BTreeMap::<String, String>::new();
    let mut declared = Vec::new();
    for entry in entries {
        let bound = match entry.kind {
            "struct" | "enum" => selected
                .selection
                .full
                .contains(&crate::crossing::bare(&entry.path)),
            "field" => selected
                .members
                .iter()
                .any(|(i, _, _)| i.path == entry.path && i.hash == entry.hash),
            "fn" | "method" => selected
                .accepted
                .iter()
                .any(|i| i.path == entry.path && i.hash == entry.hash),
            _ => false,
        };
        let reason = if !entry.cfg.is_empty() {
            Some("conditional item")
        } else if bound {
            None
        } else {
            Some(
                selected
                    .counts
                    .unbound
                    .get(&(entry.path.clone(), entry.kind, entry.hash.clone()))
                    .copied()
                    .or_else(|| selected.counts.refused.get(&entry.path).copied())
                    .unwrap_or_else(|| {
                        panic!("unexplained missing binding: {} {}", entry.kind, entry.path)
                    }),
            )
        };
        if let Some(reason) = reason {
            skipped.insert(format!("{} ({})", entry.path, entry.kind), reason.into());
        } else {
            declared.push(internals_view::Declaration {
                path: entry.path.clone(),
                hash: entry.hash.clone(),
                kind: Some(entry.kind.into()),
            });
        }
    }
    assert!(!declared.is_empty(), "coverage must not be vacuous");
    let raw: Vec<_> = selected
        .raw
        .iter()
        .enumerate()
        .map(|(index, (entry, _))| internals_view::Injection {
            target: entry.path.clone(),
            at_return: false,
            listener: format!("raw_{index}"),
        })
        .collect();
    let paths: Vec<_> = declared
        .iter()
        .map(|i| i.path.clone())
        .chain(raw.iter().map(|i| i.target.clone()))
        .collect();
    let (host, bound) = internals_table::table_with_points(entries, &paths, &game_points);
    assert!(
        bound.refused.is_empty(),
        "accepted entries lost host bindings: {:?}",
        bound.refused
    );
    // Generate now, too: an entry silently lost from selection must fail before compilation.
    println!(
        "generating universal guest view: {} declarations, {} raw records",
        declared.len(),
        raw.len()
    );
    internals_view::generate(entries, &declared, &raw, &game_points).unwrap();
    fs::write(
        stage.join("crates/horizon-client/src/__internals.rs"),
        crate::rustfmt(&host).unwrap(),
    )
    .unwrap();
    let lib = stage.join("crates/horizon-client/src/lib.rs");
    fs::write(
        &lib,
        fs::read_to_string(&lib)
            .unwrap()
            .replace("#![forbid(unsafe_code)]", "#![deny(unsafe_code)]"),
    )
    .unwrap();
    let mut manifest = String::from(
        "id='coverage'\nname='Coverage'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\n",
    );
    for d in &declared {
        manifest.push_str(&format!(
            "[[native.internals]]\npath = {:?}\nhash = {:?}\nkind = {:?}\n",
            d.path,
            d.hash,
            d.kind.as_ref().unwrap()
        ));
    }
    for injection in &raw {
        manifest.push_str(&format!(
            "[[inject]]\nid = {:?}\nat = {:?}\nkind = \"head\"\naccess = \"internals\"\nhash = \"coverage\"\nwith = {:?}\n",
            injection.listener, injection.target, injection.listener
        ));
    }
    manifest.push_str(&format!(
        "[wasm]\nsha512='{:0<128}'\ntoolchain='1.99.1'\n",
        ""
    ));
    let manifest_path = build.join("coverage.toml");
    fs::write(&manifest_path, manifest).unwrap();
    let report = format!(
        "## Internals binding coverage\n\n{} entries, {} host types, {} thunks.\n\n### Renamed methods\n\n{}\n\n### Explicit exclusions\n\n{}\n",
        declared.len(),
        bound.types,
        bound.thunks,
        bound
            .renamed_methods
            .iter()
            .map(|(p, n)| format!("- `{p}`: `{n}`"))
            .collect::<Vec<_>>()
            .join("\n"),
        skipped
            .iter()
            .map(|(p, r)| format!("- `{p}`: {r}"))
            .chain(bound.refused_map_keys.iter().map(|key| format!(
                "- map key `{key}`: native equality is not proven structural or cannot be decoded"
            )))
            .collect::<Vec<_>>()
            .join("\n")
    );
    fs::write(build.join("summary.md"), &report).unwrap();
    println!("{report}");
}

// Cargo test prepends its own toolchain's lib. Identical compiler builds can then load
// that toolchain's driver and infer its sysroot, even with an absolute RUSTC path.
fn guest_compiler(command: &mut Command, rustc: &Path, loader: Option<std::ffi::OsString>) {
    let lib = rustc.parent().unwrap().parent().unwrap().join("lib");
    let mut paths = vec![lib];
    if let Some(existing) = loader {
        paths.extend(std::env::split_paths(&existing));
    }
    command
        .env("RUSTC", rustc)
        .env("RUSTC_WRAPPER", "")
        .env("LD_LIBRARY_PATH", std::env::join_paths(paths).unwrap());
}

// Preserve unchanged mtimes for Cargo, but remove deleted sources and generated outputs.
fn sync_tree(source: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(dest).unwrap() {
        let path = entry.unwrap().path();
        let original = source.join(path.file_name().unwrap());
        if !original.exists() || original.is_dir() != path.is_dir() {
            if path.is_dir() {
                fs::remove_dir_all(path).unwrap();
            } else {
                fs::remove_file(path).unwrap();
            }
        }
    }
    for entry in fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        let target = dest.join(path.file_name().unwrap());
        if path.is_dir() {
            sync_tree(&path, &target);
        } else {
            let bytes = fs::read(&path).unwrap();
            if fs::read(&target).ok().as_deref() != Some(bytes.as_slice()) {
                fs::write(target, bytes).unwrap();
            }
        }
    }
}

#[test]
fn stage_sync_preserves_unchanged_files_and_removes_stale_files() {
    let temp = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/coverage-sync-{}", std::process::id()));
    if temp.exists() {
        fs::remove_dir_all(&temp).unwrap();
    }
    let source = temp.join("source");
    let dest = temp.join("dest");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/keep"), "same").unwrap();
    sync_tree(&source, &dest);
    let before = fs::metadata(dest.join("nested/keep"))
        .unwrap()
        .modified()
        .unwrap();
    fs::write(dest.join("stale"), "old").unwrap();
    fs::write(source.join("new"), "new").unwrap();
    sync_tree(&source, &dest);
    assert_eq!(
        before,
        fs::metadata(dest.join("nested/keep"))
            .unwrap()
            .modified()
            .unwrap()
    );
    assert!(!dest.join("stale").exists());
    assert_eq!(fs::read_to_string(dest.join("new")).unwrap(), "new");
    fs::write(source.join("nested/keep"), "changed").unwrap();
    sync_tree(&source, &dest);
    assert_eq!(
        fs::read_to_string(dest.join("nested/keep")).unwrap(),
        "changed"
    );
    fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires the coverage wasm compiler; run by wasm-mods"]
fn guest_compiler_ignores_another_toolchains_driver() {
    let rustc = std::env::var_os("HORIZON_COVERAGE_WASM_RUSTC").expect("coverage compiler");
    let rustc = Path::new(&rustc);
    let root = rustc.parent().unwrap().parent().unwrap();
    let temp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/coverage-loader");
    let other = temp.join("other/lib");
    fs::create_dir_all(&other).unwrap();
    // Same driver, different installation with no target std: the Cargo-test failure.
    for entry in fs::read_dir(root.join("lib")).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("librustc_driver")
        {
            let dest = other.join(path.file_name().unwrap());
            if !dest.exists() {
                fs::copy(path, dest).unwrap();
            }
        }
    }
    let other = other.canonicalize().unwrap();
    let polluted = std::env::join_paths([other.clone(), root.join("lib")]).unwrap();
    let wrong = checked(
        Command::new(rustc)
            .env("LD_LIBRARY_PATH", &polluted)
            .args(["--print", "sysroot"]),
    );
    assert_eq!(
        Path::new(std::str::from_utf8(&wrong.stdout).unwrap().trim()),
        other.parent().unwrap()
    );
    let source = temp.join("probe.rs");
    fs::write(&source, "pub fn probe() -> std::vec::Vec<u8> { vec![1] }").unwrap();
    let mut command = Command::new(rustc);
    guest_compiler(&mut command, rustc, Some(polluted));
    checked(
        command
            .args([
                "--crate-type=lib",
                "--emit=metadata",
                "--target=wasm32-unknown-unknown",
            ])
            .arg(&source)
            .arg("-o")
            .arg(temp.join("probe.rmeta")),
    );
    fs::remove_dir_all(temp).unwrap();
}
