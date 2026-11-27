//! One source staging pass for the launcher and `sunrise-stage`.
use crate::source::SourceLayout;
use crate::{internals, replace};
/// The build a staged copy is of, named in each build-data file's header.
pub use mod_surface::build_data::Build;
use mod_surface::build_data::Document;
use std::{path::Path, time::Instant};

/// Discovery instrumentation must never be used to build or publish a game.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Widen visibility and give functions their replacement slots,
    /// without diagnostic attributes.
    Build,
    /// Add internal-use diagnostics for an isolated metadata-only mod check.
    Discovery,
}

/// Counts and catalog produced by source staging.
pub struct Staged {
    /// Replacement slot counts.
    pub replace: replace::Staged,
    /// Exposed internal signatures and changed visibility counts.
    pub internals: internals::Widened,
}

/// Expose internals, give functions replacement slots and write the build data
/// (`internals.json`, `loops.json`, `functions.json`), each with a header naming `build`.
/// Catalog only the original game items, before adding the replacement helpers. For lower-crate
/// mixin targets, declarations from literal includes are widened in their own staged files.
/// A pass stamps the copy before edits; even a failed pass requires fresh source for a retry.
pub fn stage(
    root: &Path,
    internals_json: &Path,
    mode: Mode,
    build: &Build,
) -> Result<Staged, String> {
    stage_with_layout(root, SourceLayout::read(root)?, internals_json, mode, build)
}

pub fn stage_with_layout(
    root: &Path,
    layout: SourceLayout,
    internals_json: &Path,
    mode: Mode,
    build: &Build,
) -> Result<Staged, String> {
    let stamp = root.join(".sunrise-source-staged");
    if stamp.exists() {
        return Err("source tree is already staged; use a fresh copy of the source".into());
    }
    std::fs::write(
        &stamp,
        "Source staging started. Use a fresh copy to stage again.\n",
    )
    .map_err(|error| format!("{}: {error}", stamp.display()))?;
    // Listed before any pass rewrites the source: the markers are the source's.
    let started = Instant::now();
    #[derive(serde::Serialize)]
    struct Loops {
        loops: Vec<crate::loops::PublishedLoop>,
    }
    let loops = Loops {
        loops: crate::loops::collect_with_layout(root, layout)?,
    };
    eprintln!(
        "stage_phase=loops seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    // Indexed before any pass edits a file: its spans and hashes are of the source a mixin
    // build downloads.
    let started = Instant::now();
    let generic: std::collections::BTreeSet<String> = loops
        .loops
        .iter()
        .map(|l| crate::functions::key(&l.generic_target()))
        .collect();
    let (mut functions, pristine_owners, workspace_traits) =
        crate::functions::index_with_context(root, layout, &generic)?;
    eprintln!(
        "stage_phase=functions seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    write(
        &internals_json.with_file_name(crate::loops::FILE),
        &Document::new(build, loops),
    )?;
    let started = Instant::now();
    let mut internals = internals::stage(root, layout, matches!(mode, Mode::Discovery))?;
    internals
        .entries
        .extend(crate::crossing::foreign_internals(root)?);
    #[derive(serde::Serialize)]
    struct Internals<'a> {
        internals: &'a [internals::Internal],
    }
    write(
        internals_json,
        &Document::new(
            build,
            Internals {
                internals: &internals.entries,
            },
        ),
    )?;
    eprintln!(
        "stage_phase=internals seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let replace =
        replace::stage_with_context(root, layout, Some(pristine_owners), Some(&workspace_traits))?;
    eprintln!(
        "stage_phase=replace seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    functions.retain_slotted(&replace.slots, &generic);
    for (key, function) in &mut functions.functions {
        if let Some(expected) = &function.trait_binding
            && let Some(actual) = replace.trait_slots.get(key)
        {
            if !expected.same_binding(actual) {
                return Err(format!(
                    "trait slot evidence mismatch for {key}: index={expected:?}, staged={actual:?}"
                ));
            }
            function.trait_binding = Some(actual.clone());
        }
    }
    functions.trait_slots = replace.trait_slots.clone();
    let refused_by_reason = functions.refusal_counts();
    eprintln!(
        "function index: installers={}, selectable={}, refused={}",
        replace.functions,
        functions.functions.len(),
        functions.refused.len(),
    );
    eprintln!(
        "function index refusals: {}",
        serde_json::to_string(&refused_by_reason).expect("refusal counts serialize")
    );
    // One line: the index is the largest build-data file, and only programs read it.
    let path = internals_json.with_file_name(crate::functions::FILE);
    let text = serde_json::to_string(&Document::new(build, &functions))
        .map_err(|error| error.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Staged { replace, internals })
}

fn write(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(root: &Path, internals: &Path, mode: Mode) -> Result<Staged, String> {
        let build = Build {
            commit: "c".repeat(40),
        };
        super::stage(root, internals, mode, &build)
    }

    #[test]
    #[ignore = "daily"]
    fn three_game_domains_stage_and_patch_their_own_functions() {
        use crate::source::{GameDomain, SourceLayout};
        use crate::{functions, patch};
        use std::{fs, process::Command};

        const COMMON: GameDomain = GameDomain {
            package: "sunrise-fixture-common",
            key: "sunrise_fixture_common",
        };
        const SERVER: GameDomain = GameDomain {
            package: "sunrise-fixture-server",
            key: "sunrise_fixture_server",
        };
        const CLIENT: GameDomain = GameDomain {
            package: "sunrise-fixture-client",
            key: "sunrise_fixture_client",
        };
        const RUNTIME: GameDomain = GameDomain {
            package: "sunrise-fixture-runtime",
            key: "sunrise_fixture_runtime",
        };
        let layout = SourceLayout {
            runtime: RUNTIME,
            domains: &[COMMON, SERVER, CLIENT],
            entry: None,
        };
        let input = tempfile::tempdir().unwrap();
        let staged = tempfile::tempdir().unwrap();
        let source = input.path();
        let output = staged.path();
        let write = |path: &Path, text: &str| {
            for tree in [source, output] {
                let path = tree.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
        };
        for domain in [COMMON, SERVER, CLIENT, RUNTIME] {
            let package = domain.package_path(Path::new(""));
            let mut manifest = format!("[package]\nname = '{}'\n", domain.package);
            if domain.key == COMMON.key {
                manifest.push_str("[lib]\npath = 'engine/lib.rs'\nname = 'common_engine'\n");
            } else if domain.key != RUNTIME.key {
                manifest.push_str("[dependencies]\ncommon_engine = { package = 'sunrise-fixture-common', path = '../sunrise-fixture-common' }\n");
            } else {
                manifest.push_str("[dependencies]\nsunrise-fixture-server = { path = '../sunrise-fixture-server' }\nsunrise-fixture-client = { path = '../sunrise-fixture-client', optional = true }\n[features]\nclient = ['dep:sunrise-fixture-client']\n");
            }
            write(&package.join("Cargo.toml"), &manifest);
            let source_dir = if domain.key == COMMON.key {
                "engine"
            } else {
                "src"
            };
            let src = package.join(source_dir);
            let lib = if domain.key == RUNTIME.key {
                "#[cfg(feature = \"client\")] pub fn desktop() -> u32 { sunrise_fixture_client::game::same() }\npub fn dedicated() -> u32 { sunrise_fixture_server::game::same() }\n"
            } else {
                "#![forbid(unsafe_code)]\npub mod game;\n"
            };
            write(&src.join("lib.rs"), lib);
        }
        for (domain, game) in [
            (COMMON, "include!(\"fragment.rs\");\n"),
            (
                SERVER,
                "pub struct ServerGame { value: u32 }\nfn same() -> u32 { common_engine::game::same() + 1 }\n",
            ),
            (
                CLIENT,
                "fn same() -> u32 { common_engine::game::same() + 2 }\n",
            ),
        ] {
            let lib = domain.library(source).unwrap();
            let game_path = lib.with_file_name("game.rs");
            write(game_path.strip_prefix(source).unwrap(), game);
        }
        let fragment = "crates/sunrise-fixture-common/engine/fragment.rs";
        write(Path::new(fragment), "fn same() -> u32 { 40 }\n");
        write(
            Path::new("crates/unrelated/Cargo.toml"),
            "[package]\nname = 'unrelated'\n",
        );
        write(
            Path::new("crates/unrelated/src/lib.rs"),
            "pub fn same() -> u32 { 99 }\n",
        );
        write(Path::new("crates/sunrise-mod-api-gen/internals.txt"), "");
        write(Path::new(crate::crossing::CROSSING_FOREIGN_FILE), "[]");

        let result = stage_with_layout(
            output,
            layout,
            &output.join("internals.json"),
            Mode::Build,
            &Build {
                commit: "fixture".into(),
            },
        )
        .unwrap();
        let docs = fs::read_to_string(output.join(functions::FILE)).unwrap();
        let (_, index): (_, functions::FunctionIndex) =
            mod_surface::build_data::read(functions::FILE, &docs).unwrap();
        let targets: Vec<_> = layout
            .domains
            .iter()
            .map(|domain| format!("{}::game::same", domain.key))
            .collect();
        for target in &targets {
            assert!(
                result
                    .internals
                    .entries
                    .iter()
                    .any(|entry| entry.path == *target)
            );
        }
        assert_eq!(result.replace.crates, 3);
        assert_eq!(result.replace.slots, targets.iter().cloned().collect());
        assert_eq!(
            index
                .functions
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            result.replace.slots
        );
        assert_eq!(index.functions[&targets[0]].file, fragment);
        assert_eq!(index.crates[COMMON.key].library, "common_engine");
        assert_eq!(
            index.check_with_layout(source, "unrelated::same", layout),
            Err(("E_MIXIN_NOT_REPLACEABLE", replace::UNMARKED.into())),
        );
        for domain in layout.domains {
            assert!(
                fs::read_to_string(domain.library(output).unwrap())
                    .unwrap()
                    .contains("AtomicPtr<()>; 1]")
            );
        }

        let extras = patch::Extras {
            modules: vec![patch::SupportModule {
                id: "support".into(),
                file: "crates/sunrise-fixture-server/src/game.rs".into(),
                name: "patch_support".into(),
                text: "pub fn answer() -> u32 { 10 }".into(),
                public: true,
            }],
            bridges: vec![patch::Bridge {
                id: "value".into(),
                method: "patch_value".into(),
                kind: "GET".into(),
                target: "sunrise_fixture_server::game::ServerGame".into(),
                member: Some("value".into()),
            }],
            ..Default::default()
        };
        let patches =
            patch::generate_with_layout(source, layout, "patch", &targets, &extras, &index)
                .unwrap();
        assert_eq!(patches.libs.len(), 3);
        for (name, rust) in &patches.libs {
            let server = name == "patch_fixture_server";
            assert_eq!(rust.contains("mod patch_support"), server, "{name}");
            assert_eq!(rust.contains("pub mod __fnr_bridges"), server, "{name}");
        }
        let compile = |name: &str, input: &Path, dependencies: &[(&str, &Path)]| {
            let lib = output.join(format!("lib{name}.rlib"));
            let mut rustc = Command::new("rustc");
            rustc
                .args(["--edition=2024", "--crate-type=rlib", "--crate-name", name])
                .arg("-L")
                .arg(format!("dependency={}", output.display()))
                .arg("-o")
                .arg(&lib)
                .arg(input);
            for (name, path) in dependencies {
                rustc
                    .arg("--extern")
                    .arg(format!("{name}={}", path.display()));
            }
            let result = rustc.output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            lib
        };
        let common = compile("common_engine", &COMMON.library(output).unwrap(), &[]);
        let server = compile(
            SERVER.key,
            &SERVER.library(output).unwrap(),
            &[("common_engine", &common)],
        );
        let runtime = compile(
            RUNTIME.key,
            &RUNTIME.library(output).unwrap(),
            &[(SERVER.key, &server)],
        );
        let files = patches
            .libs
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        let patch_source = output.join("patch_fixture_server.rs");
        fs::write(&patch_source, &files["patch_fixture_server"]).unwrap();
        let patch = compile(
            "patch_fixture_server",
            &patch_source,
            &[(SERVER.key, &server), ("common_engine", &common)],
        );
        let runner = output.join("dedicated.rs");
        fs::write(&runner, "fn main() { assert_eq!(sunrise_fixture_runtime::dedicated(), 41); assert_eq!(patch_fixture_server::game::patch_support::answer(), 10); }").unwrap();
        let binary = output.join(format!("dedicated{}", std::env::consts::EXE_SUFFIX));
        let result = Command::new("rustc")
            .args(["--edition=2024", "-L"])
            .arg(format!("dependency={}", output.display()))
            .arg("--extern")
            .arg(format!("{}={}", RUNTIME.key, runtime.display()))
            .arg("--extern")
            .arg(format!("patch_fixture_server={}", patch.display()))
            .arg("-o")
            .arg(&binary)
            .arg(runner)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(Command::new(binary).status().unwrap().success());
    }

    #[test]
    #[ignore = "daily"]
    fn lower_mixin_copy_calls_private_included_helper() {
        use crate::{functions, patch};
        use std::{fs, process::Command};

        let target_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        fs::create_dir_all(&target_dir).unwrap();
        let scratch = tempfile::tempdir_in(target_dir).unwrap();
        let out = scratch.path();
        let pristine = out.join("pristine");
        let staged = out.join("staged");
        let lower_src = Path::new("crates/lower/src");
        let selected = lower_src.join("hidden.rs");
        let helper = lower_src.join("fragments/nested/helper.rs");
        let write = |root: &Path, relative: &Path, text: &str| {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let source = [
            (
                "crates/sunrise-client/Cargo.toml",
                "[package]\nname = 'sunrise-client'\n[dependencies]\n",
            ),
            ("crates/sunrise-client/src/lib.rs", "mod game; mod entry;"),
            ("crates/sunrise-client/src/game.rs", ""),
            ("crates/sunrise-client/src/entry.rs", ""),
            ("crates/sunrise-mod-api-gen/internals.txt", ""),
            (crate::crossing::CROSSING_FOREIGN_FILE, "[]"),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"lower\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n",
            ),
            (
                "crates/lower/Cargo.toml",
                "[package]\nname = 'lower'\nversion = '0.1.0'\n",
            ),
            ("crates/lower/src/lib.rs", "mod hidden;"),
        ];
        let marker = "#[sunrise_mod_api_macros::replaceable]";
        let marked = format!(
            r#"
{marker}
fn listed(value: u32) -> u32 {{ child::deep(imported(value)) - 1 + pick(0) }}
include!("fragments/outer.rs");
#[cfg(unix)]
include!("fragments/unix.rs");
#[cfg_attr(unix, cfg(not(unix)))]
include!("fragments/other.rs");
mod child {{ include!("fragments/child.rs"); }}
#[cfg(feature = "fast")]
include!("fragments/fast.rs");
#[cfg(not(feature = "fast"))]
include!("fragments/slow.rs");
fn fast_impl(value: u32) -> u32 {{ value }}
fn slow_impl(value: u32) -> u32 {{ value + 1000 }}
"#
        );
        for root in [&pristine, &staged] {
            for (relative, text) in source {
                write(root, Path::new(relative), text);
            }
            write(root, &selected, &marked);
            write(
                root,
                &lower_src.join("fragments/outer.rs"),
                r#"include!("nested/helper.rs");"#,
            );
            write(
                root,
                &helper,
                "fn helper(value: u32) -> u32 { value * 7 }\nuse self::helper as imported;",
            );
            write(
                root,
                &lower_src.join("fragments/unix.rs"),
                r#"include!("nested/unix.rs");"#,
            );
            write(
                root,
                &lower_src.join("fragments/nested/unix.rs"),
                "fn unix_only() {}\nuse std::path::Path as PlatformPath;",
            );
            // A helper only the inline child's fragment declares, visible to its parent.
            write(
                root,
                &lower_src.join("fragments/child.rs"),
                "pub(super) fn deep(value: u32) -> u32 { value + 1 }",
            );
            // The lower crate is built with `fast`, the patch without: the copy must still
            // call what the prebuilt crate's import chose.
            write(
                root,
                &lower_src.join("fragments/fast.rs"),
                "use self::fast_impl as pick;",
            );
            write(
                root,
                &lower_src.join("fragments/slow.rs"),
                "use self::slow_impl as pick;",
            );
            write(
                root,
                &lower_src.join("fragments/other.rs"),
                "fn non_unix_only() {}\nuse std::path::PathBuf as PlatformPath;",
            );
        }

        stage(&staged, &staged.join("internals.json"), Mode::Build).unwrap();
        let text = fs::read_to_string(staged.join(functions::FILE)).unwrap();
        let (_, index): (Build, functions::FunctionIndex) =
            mod_surface::build_data::read(functions::FILE, &text).unwrap();
        let index = index.checked().unwrap();
        let targets = ["lower::hidden::listed".to_owned()];
        let generate = || {
            patch::generate(
                &pristine,
                "fixture_patch",
                &targets,
                &Default::default(),
                &index,
            )
        };
        let generated = generate().unwrap();
        let [(patch_name, patch_source)] = generated.libs.as_slice() else {
            panic!("expected exactly one lower-crate patch library");
        };
        let install = generated.patched[0].install.as_deref().unwrap();

        // The fixture links with rustc alone, so remove only the original no-op marker from
        // the staged file. The generated slot and all visibility edits are left as staged.
        let text = fs::read_to_string(staged.join(&selected)).unwrap();
        fs::write(staged.join(&selected), text.replace(marker, "")).unwrap();
        let rustc = |source: &Path, args: &[&str]| {
            let result = Command::new("rustc")
                .args(["--edition=2024", "--out-dir"])
                .arg(out)
                .args(args)
                .arg(source)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}: {}",
                source.display(),
                String::from_utf8_lossy(&result.stderr)
            );
        };
        rustc(
            &staged.join(lower_src).join("lib.rs"),
            &[
                "--crate-type=rlib",
                "--crate-name=lower",
                "--cfg",
                "feature=\"fast\"",
            ],
        );
        let lower = format!("lower={}", out.join("liblower.rlib").display());
        let patch_file = out.join("patch.rs");
        fs::write(&patch_file, patch_source).unwrap();
        rustc(
            &patch_file,
            &[
                "--crate-type=rlib",
                "--crate-name",
                patch_name,
                "--extern",
                &lower,
            ],
        );
        let patch = format!(
            "{patch_name}={}",
            out.join(format!("lib{patch_name}.rlib")).display()
        );
        let search = format!("dependency={}", out.display());
        write(
            out,
            Path::new("driver.rs"),
            &format!(
                r#"
fn main() {{
    assert_eq!({patch_name}::hidden::listed(6), 42);
    assert_eq!({patch_name}::hidden::helper(6), 42);
    assert!({install});
    assert_eq!(lower::hidden::listed(6), 42);
}}
"#
            ),
        );
        rustc(
            &out.join("driver.rs"),
            &["--extern", &lower, "--extern", &patch, "-L", &search],
        );
        let result = Command::new(out.join(format!("driver{}", std::env::consts::EXE_SUFFIX)))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );

        // The target and its span are unchanged. The mirror must still read and validate the
        // fragment by its indexed path, and give a source diagnostic if it disappears.
        fs::remove_file(pristine.join(&helper)).unwrap();
        let refused = generate().unwrap_err();
        assert_eq!(refused[0].code, "E_MIXIN_SOURCE");
        assert!(refused[0].why.contains("helper.rs"), "{}", refused[0].why);
    }

    #[test]
    fn staging_writes_catalogs_and_slots_and_refuses_reuse() {
        // Each iteration starts from fresh source, not the preceding staged fixture.
        for copy in 0..3 {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().to_path_buf();
            let source = root.join("crates/sunrise-client/src/game.rs");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(
                &source,
                "impl Game<'_> { fn unit(&self) {} fn optional(&self) -> Option<u32> { None } }",
            )
            .unwrap();
            std::fs::write(source.parent().unwrap().join("entry.rs"), "").unwrap();
            std::fs::write(
                source.parent().unwrap().join("lib.rs"),
                "mod game; mod entry;\n",
            )
            .unwrap();
            std::fs::write(
                root.join("crates/sunrise-client/Cargo.toml"),
                "[package]\nname = 'sunrise-client'\n[dependencies]\n",
            )
            .unwrap();
            std::fs::write(
                root.join("Cargo.lock"),
                "version = 4\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
            let lists = root.join("crates/sunrise-mod-api-gen");
            std::fs::create_dir_all(&lists).unwrap();
            std::fs::write(lists.join("internals.txt"), "").unwrap();
            std::fs::write(root.join(crate::crossing::CROSSING_FOREIGN_FILE), "[]").unwrap();
            let output = root.join("internals.json");
            let functions = root.join(crate::functions::FILE);
            if copy == 2 {
                // Refuse retries even when a late output failure leaves edited source.
                std::fs::create_dir(&functions).unwrap();
                assert!(stage(&root, &output, Mode::Build).is_err());
            } else {
                stage(&root, &output, Mode::Build).unwrap();
                let bytes = std::fs::read(&functions).unwrap();
                assert_eq!(bytes.last(), Some(&b'\n'));
                // Every build-data file opens with the header naming the build.
                for file in mod_surface::build_data::FILES {
                    let text = std::fs::read_to_string(root.join(file)).unwrap();
                    let (build, _): (Build, serde_json::Value) =
                        mod_surface::build_data::read(file, &text).unwrap();
                    assert_eq!(build.commit, "c".repeat(40), "{file}");
                }
            }
            let before = std::fs::read(&source).unwrap();
            let text = String::from_utf8_lossy(&before);
            // Even a failure writing the last catalog leaves this copy with replacement slots.
            assert!(text.contains("__fnr_unit") && text.contains("__fnr_optional"));
            for mode in [Mode::Build, Mode::Discovery] {
                let error = stage(&root, &output, mode)
                    .err()
                    .expect("staging twice must fail");
                assert_eq!(
                    error,
                    "source tree is already staged; use a fresh copy of the source"
                );
                assert_eq!(std::fs::read(&source).unwrap(), before);
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn staging_keeps_reparented_restricted_trait_and_glob_owner_slots() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let write = |relative: &str, text: &str| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "crates/sunrise-client/Cargo.toml",
            "[package]\nname = 'sunrise-client'\n[dependencies]\n",
        );
        write("crates/sunrise-client/src/lib.rs", "mod game; mod entry;\n");
        write("crates/sunrise-client/src/entry.rs", "");
        write(
            "crates/sunrise-client/src/game.rs",
            "mod boats;\n#[path = \"game/server_side.rs\"]\npub(in crate::game) mod server_side;\npub(in crate::game) use server_side::entity;\n",
        );
        write(
            "crates/sunrise-client/src/game/boats.rs",
            "pub(in crate::game) struct Boat;\n",
        );
        write(
            "crates/sunrise-client/src/game/server_side.rs",
            "pub(in crate::game::server_side) use super::*;\n#[path = \"entity.rs\"]\npub(in crate::game) mod entity;\n",
        );
        write(
            "crates/sunrise-client/src/game/entity.rs",
            "use super::boats::Boat;\n\
             pub(in crate::game) trait Entity { fn bounds(&self) -> u32; }\n\
             impl Entity for Boat { fn bounds(&self) -> u32 { 7 } }\n\
             impl std::fmt::Debug for Boat { fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } }\n",
        );
        write(
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n",
        );
        write("crates/sunrise-mod-api-gen/internals.txt", "");
        write(crate::crossing::CROSSING_FOREIGN_FILE, "[]");

        let staged = stage(root, &root.join("internals.json"), Mode::Build).unwrap();
        let key = "sunrise_client::game::server_side::entity::<Boat as Entity>::bounds";
        assert!(
            staged.replace.slots.contains(key),
            "{:?}",
            staged.replace.slots
        );
        let entity =
            std::fs::read_to_string(root.join("crates/sunrise-client/src/game/entity.rs")).unwrap();
        assert!(entity.contains("pub fn __fnr_as_Entity_bounds"), "{entity}");
        let text = std::fs::read_to_string(root.join(crate::functions::FILE)).unwrap();
        let (_, index): (Build, crate::functions::FunctionIndex) =
            mod_surface::build_data::read(crate::functions::FILE, &text).unwrap();
        let expected_binding = replace::TraitSlotEvidence {
            owner: replace::ResolvedItem {
                module: "sunrise_client::game::boats".into(),
                item: "Boat".into(),
            },
            trait_decl: replace::ResolvedItem {
                module: "sunrise_client::game::server_side::entity".into(),
                item: "Entity".into(),
            },
            installer_module: "sunrise_client::game::server_side::entity".into(),
            installer: "__fnr_as_Entity_bounds".into(),
            reader: Some("__fnr_read_slot_0".into()),
            cell: Some(0),
        };
        assert_eq!(
            index
                .functions
                .get(key)
                .and_then(|f| f.trait_binding.as_ref()),
            Some(&expected_binding),
            "{:?}",
            index.functions.get(key)
        );
        assert_eq!(index.trait_slots.get(key), Some(&expected_binding));
        assert_eq!(
            index
                .refusal_counts()
                .get("its trait implementation has no replacement installer (foreign trait)"),
            Some(&1)
        );
    }
}
