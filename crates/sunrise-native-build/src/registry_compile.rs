//! Cargo builds each mod as a Rust dylib; the SDK executable remains unchanged.
use super::{
    api, dependencies, internals, prebuilt, registry_plan, registry_rustc, registry_shared,
};
use crate::{
    host::{tr, trf},
    rust::Toolchain,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct Options<'a> {
    pub consent: Option<&'a dyn Fn(&super::RegistryInventory) -> bool>,
    pub adapter: &'a Path,
    pub mixins: &'a super::MixinsIdentity,
    /// The package identity and internals already checked by `prebuilt::compile`.
    pub sdk: &'a prebuilt::Identity,
    pub catalog: &'a internals::Catalog,
}

fn compiler_plan(
    rust: &dyn Toolchain,
    game: &Path,
    bindings: BTreeMap<PathBuf, registry_rustc::Bindings>,
) -> registry_rustc::Plan {
    registry_rustc::Plan {
        rustc: rust.rustc(),
        target: rust.target().into(),
        cache_wrapper: std::env::var_os("RUSTC_WRAPPER").map(PathBuf::from),
        workspace_wrapper: std::env::var_os("RUSTC_WORKSPACE_WRAPPER").map(PathBuf::from),
        // Transitive proc-macro metadata lives in host, just as on the direct SDK route.
        search: vec![game.join("deps"), game.join("host")],
        native: game.join("native"),
        crates: bindings,
    }
}

const LOGGERS: &str = "/* MOD_LOGGERS */";

fn check_log_template(template: &str, native_log: bool) -> Result<(), String> {
    if native_log && template.matches(LOGGERS).count() != 1 {
        return Err(trf(
            "The prebuilt SDK declares native log forwarding, but {file} must contain exactly one {marker} slot.",
            &[("file", super::api_route::TEMPLATE), ("marker", LOGGERS)],
        ));
    }
    Ok(())
}

fn add_facade_dependencies(
    project: &api::Project<'_>,
    manifest: &mut toml::Table,
    facade_relative: &str,
    facade_package: &str,
) -> Result<(), String> {
    let dependencies = manifest
        .entry("dependencies")
        .or_insert_with(|| toml::Table::new().into())
        .as_table_mut()
        .ok_or("invalid dependencies")?;
    for (alias, library) in project.api_dependencies() {
        if library != "internals" {
            continue;
        }
        // Cargo names the extern crate after the key with hyphens as underscores.
        if let Some(existing) = dependencies
            .keys()
            .find(|key| key.replace('-', "_") == alias.replace('-', "_"))
        {
            return Err(trf(
                "{mod}: Cargo.toml dependency alias `{alias}` is already in use; rename the dependency or give `sunrise-internals` a different alias.",
                &[("mod", &project.bundle.manifest.id), ("alias", existing)],
            ));
        }
        dependencies.insert(
            alias,
            toml::toml! {
                path = (format!("../../{facade_relative}"))
                package = facade_package
            }
            .into(),
        );
    }
    Ok(())
}

/// Add the SDK entry and give Cargo the same checked root used for generation.
fn stage_library(
    dest: &Path,
    manifest: &mut toml::Table,
    id: &str,
    entry: &str,
) -> Result<PathBuf, String> {
    let lib = manifest
        .entry("lib")
        .or_insert_with(|| toml::Table::new().into())
        .as_table_mut()
        .ok_or(tr("Invalid lib table"))?;
    let root = prebuilt::stage_entry(dest, lib.get("path"), id, entry)?;
    lib.insert("path".into(), root.display().to_string().into());
    lib.insert(
        "crate-type".into(),
        vec![toml::Value::String("dylib".into())].into(),
    );
    Ok(dest.join(root))
}

/// `out` is the mods folder's staging folder, rewritten every build; `target_dir` is cargo's build
/// folder, kept between builds of the same game build (see `prebuilt::compile`).
#[allow(clippy::too_many_arguments)]
pub(super) fn stage_and_cargo(
    rust: &dyn Toolchain,
    game: &Path,
    out: &Path,
    target_dir: &Path,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    options: Options<'_>,
    check_only: bool,
) -> Result<Option<(PathBuf, Vec<PathBuf>)>, String> {
    let sdk = options.sdk;
    let template = api::template(&game.join(super::api_route::TEMPLATE))?;
    check_log_template(&template, sdk.native_log)?;
    let mut patches = sdk.registry_patches.clone();
    let sdk_root = game.canonicalize().map_err(|e| e.to_string())?;
    for (_, registry) in patches.iter_mut() {
        for (_, spec) in registry
            .as_table_mut()
            .ok_or_else(|| tr("Invalid SDK patch registry").to_string())?
            .iter_mut()
        {
            let spec = spec
                .as_table_mut()
                .ok_or_else(|| tr("Invalid SDK patch").to_string())?;
            let relative = spec
                .get("path")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| tr("SDK patch has no path").to_string())?;
            let path = sdk_root.join(relative).canonicalize().map_err(|e| {
                trf(
                    "SDK patch {path}: {error}",
                    &[("path", relative), ("error", &e.to_string())],
                )
            })?;
            if !path.starts_with(&sdk_root) {
                return Err(tr("SDK patch escapes the verified SDK").into());
            }
            spec.insert("path".into(), path.to_string_lossy().into_owned().into());
        }
    }
    let generated = options
        .catalog
        .prepare(projects.iter().map(|project| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    // The last build's staging, kept aside until this one's is written: a file whose bytes didn't
    // change gets its old modified time back, so cargo's fingerprints reuse what it built from it.
    let previous = out.with_extension("old");
    if previous.exists() {
        fs::remove_dir_all(&previous).map_err(|e| e.to_string())?;
    }
    if out.exists() {
        fs::rename(out, &previous).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut bindings = BTreeMap::new();
    let mut members = Vec::new();
    let mut wrapper_deps = toml::Table::new();
    let deps = game.join("deps");
    let link_args: Vec<_> = prebuilt::dylib_rpath_args(std::env::consts::OS)
        .map(str::to_owned)
        .collect();
    // Each mod's staged folder and library root, for the shared registry library.
    let mut mod_dirs = BTreeMap::new();
    let mut mod_roots = BTreeMap::new();
    // Each mod's source digest and the native mods it links, by mod id.
    let mut sources: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    let packages: BTreeMap<_, _> = resolution
        .order
        .iter()
        .map(|&index| {
            let project = &projects[index];
            (
                project.bundle.manifest.id.as_str(),
                project.package.as_str(),
            )
        })
        .collect();
    let mut wanted = Vec::new();
    let mut generated_facades = BTreeMap::new();
    for (position, &index) in resolution.order.iter().enumerate() {
        let project = &projects[index];
        let bundle = project.bundle;
        let id = &bundle.manifest.id;
        let relative = format!("native-api/{id}");
        let dest = out.join(&relative);
        super::copy_bundle(bundle, &dest)?;
        let (mut manifest, requests) =
            super::crates::rewrite_closure(project, &dest, |local, manifest| {
                fs::write(dest.join(local).join("Cargo.toml"), manifest.to_string())
                    .map_err(|e| e.to_string())
            })?;
        // Only the builder creates cross-mod paths, from declared edges and actual stage members.
        // The facade edge below likewise points to the exact package and path generated for this mod.
        let targets: Vec<_> = declared[index]
            .iter()
            .map(|dependency| dependencies::Package {
                dependency,
                package: packages.get(dependency.id.as_str()).copied(),
            })
            .collect();
        let enabled =
            dependencies::rewrite(id, &mut manifest, &targets).map_err(|e| e.to_string())?;
        let mut dependency =
            toml::toml! { path = (relative.clone()) package = (project.package.clone()) };
        if !enabled.is_empty() {
            dependency.insert(
                "features".into(),
                enabled
                    .into_iter()
                    .map(toml::Value::String)
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        wrapper_deps.insert(format!("mod_{position}"), dependency.into());
        let source = super::source_digest(bundle).ok_or(tr("Could not hash native mod inputs"))?;
        let linked = targets
            .iter()
            .filter(|target| target.package.is_some())
            .map(|target| target.dependency.id.clone())
            .collect();
        sources.insert(id.clone(), (source, linked));
        // The identity is written once cargo has resolved the graph it names.
        let root = stage_library(
            &dest,
            &mut manifest,
            id,
            &prebuilt::library_entry(
                &template,
                &[&bundle.manifest],
                prebuilt::IDENTITY_MARKER,
                &[],
            ),
        )?;
        mod_dirs.insert(id.clone(), dest.clone());
        mod_roots.insert(id.clone(), root);
        let mut externs: BTreeMap<String, PathBuf> = BTreeMap::from([
            ("mod_api".into(), prebuilt::library(&deps, "mod_api")?),
            ("mod_vocab".into(), prebuilt::library(&deps, "mod_vocab")?),
        ]);
        for (name, path) in &externs {
            prebuilt::refuse_closure_rlib(id, name, path)?;
        }
        for (name, package) in project.api_dependencies() {
            if package != "internals" {
                let path = prebuilt::library(&deps, package)?;
                prebuilt::refuse_closure_rlib(id, package, &path)?;
                externs.insert(name, path);
            }
        }
        if internals::needed(&bundle.manifest) || project.uses_internals() {
            let facade = &generated[id.as_str()];
            let facade_relative = internals::relative(id);
            let facade_package = internals::package(id);
            add_facade_dependencies(project, &mut manifest, &facade_relative, &facade_package)?;
            let facade_dir = out.join(&facade_relative);
            fs::create_dir_all(facade_dir.join("src")).map_err(|e| e.to_string())?;
            fs::write(facade_dir.join("src/lib.rs"), &facade.source).map_err(|e| e.to_string())?;
            let facade_manifest = toml::toml! { [package] name = (facade_package.clone()) version = "0.0.0" edition = "2024" };
            fs::write(facade_dir.join("Cargo.toml"), facade_manifest.to_string())
                .map_err(|e| e.to_string())?;
            let mut facade_externs = BTreeMap::new();
            for name in facade
                .crates
                .iter()
                .map(String::as_str)
                .chain(["sunrise_client"])
            {
                let path = prebuilt::facade_library(&deps, name, sdk)?;
                prebuilt::refuse_closure_rlib(id, name, &path)?;
                facade_externs.insert(name.to_owned(), path);
            }
            bindings.insert(
                facade_dir,
                registry_rustc::Bindings {
                    externs: facade_externs,
                    link_args: Vec::new(),
                },
            );
            members.push(facade_relative.clone());
            generated_facades.insert(facade_package.clone(), project.package.as_str());
            wrapper_deps.insert(
                format!("mod_{position}_internals"),
                toml::toml! { path = facade_relative package = facade_package }.into(),
            );
        }
        fs::write(dest.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
        wanted.extend(requests);
        bindings.insert(
            dest,
            registry_rustc::Bindings {
                externs,
                link_args: link_args.clone(),
            },
        );
        members.push(relative);
    }
    // `[[native.mixin]]` rows: the patched crates join the workspace beside the mods, whose
    // crates their helpers call, and compile against the SDK like a mod.
    let patch = super::patch::prepare(
        rust,
        game,
        &out.join("fnr"),
        projects,
        sdk,
        options.mixins,
        check_only,
    )?;
    if let Some(patch) = &patch {
        for name in &patch.names {
            let owner = &patch.owners[name];
            let game_features = sdk.patch_features(owner)?;
            let dir = patch.dir.join(name);
            let mut manifest = toml::toml! { [package] name = (name.clone()) version = "0.0.0" edition = "2024" [lib] path = "src/lib.rs" };
            let mut dependencies = toml::Table::new();
            for bundle in &patch.bundles {
                let id = &bundle.manifest.id;
                let project = projects
                    .iter()
                    .find(|project| project.bundle.manifest.id == *id)
                    .expect("a prepared mod is a project");
                let krate = project.crate_name();
                dependencies.insert(
                    krate,
                    toml::toml! { path = (out.join("native-api").join(id).display().to_string()) package = (project.package.clone()) }
                        .into(),
                );
            }
            manifest.insert("dependencies".into(), dependencies.into());
            let features: toml::Table = game_features
                .iter()
                .map(|feature| (feature.clone(), toml::Value::Array(Vec::new())))
                .collect();
            manifest.insert("features".into(), features.into());
            fs::write(dir.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
            let externs = super::patch::sdk_externs(game, sdk, owner)?;
            for (krate, path) in &externs {
                prebuilt::refuse_closure_rlib("sunrise_patch", krate, path)?;
            }
            bindings.insert(
                dir,
                registry_rustc::Bindings {
                    externs,
                    link_args: Vec::new(),
                },
            );
            let relative = format!(
                "fnr/{}/{name}",
                patch
                    .dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            );
            members.push(relative.clone());
            wrapper_deps.insert(
                name.clone(),
                toml::toml! { path = (relative) features = (game_features.iter().cloned().map(toml::Value::String).collect::<Vec<_>>()) }.into(),
            );
        }
        // The tool's workspace files would make the crates a second workspace inside this one.
        for file in ["Cargo.toml", "Cargo.lock"] {
            let _ = fs::remove_file(patch.dir.join(file));
        }
    }
    fs::create_dir_all(out.join("src")).map_err(|e| e.to_string())?;
    let installs = patch
        .as_ref()
        .map(|patch| patch.installs.as_slice())
        .unwrap_or(&[]);
    let mut entry = prebuilt::library_entry(&template, &[], prebuilt::IDENTITY_MARKER, installs);
    entry.push_str(prebuilt::PATCH_HOST_LINK);
    fs::write(out.join("src/lib.rs"), entry).map_err(|e| e.to_string())?;
    let mut manifest = toml::toml! { [package] name = "sunrise_patch" version = "0.0.0" edition = "2024" [lib] crate-type = ["dylib"] [workspace] resolver = "2" };
    manifest
        .get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
        .expect("generated workspace")
        .insert(
            "members".into(),
            members
                .into_iter()
                .map(toml::Value::String)
                .collect::<Vec<_>>()
                .into(),
        );
    manifest.insert("dependencies".into(), wrapper_deps.into());
    fs::write(out.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
    let mut plan = registry_plan::Plan::read(&sdk.registry_lock, projects, &wanted)?;
    plan.authorize_game_patches(&patches);
    plan.fetch_with_patches(rust, out, &patches)?;
    let mut metadata = super::registry_inventory::read(rust, out)?;
    // When two or more mods link registry crates, those crates, as cargo just resolved them, come
    // from one library, so a mod depending on several mods doesn't reach a copy of each through
    // each of them (`registry_shared`). The mods' manifests were checked above without it; it adds
    // no package cargo hasn't already resolved and locked.
    // The SDK owns the entry template and the sink it can call. The mod's lock still owns log.
    let logs = if sdk.native_log {
        metadata.runtime_logs()?
    } else {
        Default::default()
    };
    let shared = registry_shared::Shared::select(metadata.runtime_uses(out, &mod_dirs)?, logs);
    if let Some(shared) = &shared {
        shared.check_name(&metadata.packages, out)?;
        shared.write(out)?;
        for id in &shared.users {
            let path = mod_dirs[id].join("Cargo.toml");
            let mut manifest: toml::Table = fs::read_to_string(&path)
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e| format!("Cargo.toml: {e}"))?;
            let root = &mod_roots[id];
            let mut source = fs::read_to_string(root).map_err(|e| e.to_string())?;
            registry_shared::link(&mut manifest, &mut source).map_err(|e| format!("{id}: {e}"))?;
            fs::write(&path, manifest.to_string()).map_err(|e| e.to_string())?;
            fs::write(root, source).map_err(|e| e.to_string())?;
            if !shared.logs.is_empty() {
                let path = root.with_file_name("__sunrise_entry.rs");
                let entry = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                fs::write(
                    path,
                    entry.replace(
                        LOGGERS,
                        "extern crate sunrise_registry_shared;\n    sunrise_registry_shared::install_loggers();",
                    ),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let path = out.join("Cargo.toml");
        let mut manifest: toml::Table = fs::read_to_string(&path)
            .map_err(|e| e.to_string())?
            .parse()
            .map_err(|e| format!("Cargo.toml: {e}"))?;
        manifest
            .get_mut("workspace")
            .and_then(|workspace| workspace.get_mut("members"))
            .and_then(toml::Value::as_array_mut)
            .ok_or("generated workspace")?
            .push(registry_shared::RELATIVE.into());
        fs::write(&path, manifest.to_string()).map_err(|e| e.to_string())?;
        // Link it like a mod (dynamic, with the same rpath). Only the forwarder calls the SDK.
        let mut externs = BTreeMap::new();
        if !shared.logs.is_empty() {
            let api = prebuilt::library(&deps, "mod_api")?;
            prebuilt::refuse_closure_rlib(registry_shared::CRATE, "mod_api", &api)?;
            externs.insert("mod_api".into(), api);
        }
        bindings.insert(
            out.join(registry_shared::RELATIVE),
            registry_rustc::Bindings {
                externs,
                link_args: link_args.clone(),
            },
        );
        // Cargo records the new local package in the lock, offline: every crate it names is
        // already resolved there, at that exact version.
        metadata = super::registry_inventory::read(rust, out)?;
    }
    let resolved = super::read_lock(out)?;
    plan.check_resolved(&super::registry_lock::RegistryLock::parse(&resolved)?)?;
    // One workspace unifies the crates.io graph of the whole install set: another mod can move a
    // version or enable a feature under a mod whose files did not change, so each library's
    // identity names its graph as cargo just resolved it, after its source digest.
    let graphs = super::registry_inventory::graphs(rust, out, &mod_dirs)?;
    let mut identities = super::library_record::Identities::default();
    let fill = |path: &Path, identity: &str| -> Result<(), String> {
        let entry = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        fs::write(path, entry.replace(prebuilt::IDENTITY_MARKER, identity))
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    for &index in &resolution.order {
        let manifest = &projects[index].bundle.manifest;
        let (source, linked) = &sources[&manifest.id];
        let identity = identities.next(
            sdk.commit(),
            &manifest.id,
            &manifest.version.to_string(),
            &format!("{source}+{}", graphs[&manifest.id]),
            linked.iter().map(String::as_str),
        );
        fill(
            &mod_roots[&manifest.id].with_file_name("__sunrise_entry.rs"),
            &identity,
        )?;
    }
    let identity = identities.next(
        sdk.commit(),
        "sunrise_patch",
        "0",
        "none",
        patch
            .iter()
            .flat_map(|patch| &patch.bundles)
            .map(|bundle| bundle.manifest.id.as_str()),
    );
    fill(&out.join("src/lib.rs"), &identity)?;
    let mut game_names = plan.game.game_package_names();
    for registry in patches.values() {
        for name in registry.as_table().expect("validated SDK patches").keys() {
            game_names.remove(name);
        }
    }
    super::crates::check_game_graph_names(projects, &generated_facades, &resolved, &game_names)?;
    let inventory = metadata.inventory(&plan, &sdk.native_links, &mod_dirs)?;
    // The mods' own build scripts run under the same yes as the registry crates', so a mod with a
    // build script and no registry crate still asks.
    if (inventory.added_crates > 0 || !inventory.build_scripts.is_empty())
        && !options.consent.is_some_and(|ask| ask(&inventory))
    {
        return Err(crate::host::tr("The mods' build scripts and crates.io dependencies were not compiled: the plan's \"registry\" field is false or the question was declined. \"registry\": true approves compiling them; the mods' own build scripts, their dependencies' build scripts and proc macros then run on this computer").into());
    }
    let externs = prebuilt::wrapper_externs(&deps)?;
    for (name, path) in &externs {
        prebuilt::refuse_closure_rlib("sunrise_patch", name, path)?;
    }
    bindings.insert(
        out.to_path_buf(),
        registry_rustc::Bindings { externs, link_args },
    );
    if previous.exists() {
        keep_unchanged_times(out, &previous).map_err(|e| e.to_string())?;
        fs::remove_dir_all(&previous).map_err(|e| e.to_string())?;
    }
    let mut command = super::cargo(rust, out, if check_only { "check" } else { "build" });
    command
        .args([
            "--workspace",
            "--lib",
            "--offline",
            "--locked",
            "--release",
            "--target",
            rust.target(),
        ])
        .arg("--target-dir")
        .arg(target_dir)
        // Not `json-render-diagnostics`: Cargo would print every diagnostic again on its standard
        // error, at the staged files. The failure shows them from the messages, at the author's.
        .arg("--message-format=json");
    // Cargo's per-crate timings (`target/cargo-timings/`), for the check that measures where a
    // registry build's time goes; a player's kept build folder doesn't collect them.
    if std::env::var_os("SUNRISE_NATIVE_CARGO_TIMINGS").is_some() {
        command.arg("--timings");
    }
    let wrapper = compiler_plan(rust, game, bindings);
    wrapper.configure(&mut command, &out.join("rustc-plan.json"), options.adapter)?;
    let messages = out.join("cargo-messages.jsonl");
    let result = super::run_checked(
        command,
        if check_only {
            tr("Checking native mod libraries")
        } else {
            tr("Compiling registry mod libraries")
        },
        Some(fs::File::create(&messages).map_err(|e| e.to_string())?),
        None,
    );
    let outputs = super::registry_runtime::Outputs::read(&messages)?;
    if let Some(patch) = &patch {
        patch.log()?;
    }
    super::append_log(&fs::read_to_string(&messages).map_err(|e| e.to_string())?)?;
    // What the compiler printed, as a readable part of the log (the messages above, unmapped).
    for diagnostic in &outputs.errors {
        super::append_log(diagnostic.rendered().unwrap_or_default())?;
    }
    if let Err(mut error) = result {
        // The compiler runs in the build's folder: its relative file names are relative to it.
        let sources = super::present::Sources::new(
            out.to_path_buf(),
            projects
                .iter()
                .map(|project| super::present::Staged {
                    root: out.join("native-api").join(&project.bundle.manifest.id),
                    author: project.bundle.dir.clone(),
                })
                .collect(),
        );
        let mut raw = String::new();
        let mut explained = false;
        for diagnostic in &outputs.errors {
            error.push('\n');
            error.push_str(&sources.text(diagnostic));
            raw.push_str(diagnostic.rendered().unwrap_or_default());
            // Only this route builds registry crates beside the SDK's, and only a copy of one of
            // those is a different crate from the SDK's; the direct route has none to explain.
            if !explained && diagnostic.crate_present_twice() {
                explained = true;
                error.push_str(&format!("\n{}\n", crate::host::tr("SDK values keep the SDK's Rust crate identities. A registry-built copy of a trait or type is not interchangeable, even at the same version. Type mismatches are refused; SDK libraries are never substituted.")));
            }
        }
        // The linker's and Application Control's words are in the compiler's messages, no
        // longer on Cargo's standard error.
        let (blocked, libraries) = (
            crate::rust::blocked_by_app_control(&raw),
            raw.lines().any(crate::rust::missing_system_libraries),
        );
        return Err(super::advised(error, blocked, libraries));
    }
    if check_only {
        return Ok(None);
    }
    let binary = game.join("bin").join(crate::CLIENT);
    let release = target_dir.join(rust.target()).join("release");
    let mut runtime = prebuilt::runtime_libraries(rust, sdk, game)?;
    for &index in &resolution.order {
        let project = &projects[index];
        let library = release.join(prebuilt::dylib_name(&project.crate_name()));
        super::library_record::validate_exports(&library, &project.bundle.manifest.id)?;
        let id = &project.bundle.manifest.id;
        super::library_record::stamp(&library, id, identities.recorded(id), false)?;
        runtime.push(library);
    }
    if shared.is_some() {
        // Installed beside the mods with no identity record: a library they load, not a mod.
        let library = release.join(prebuilt::dylib_name(registry_shared::CRATE));
        super::library_record::validate_exports(&library, registry_shared::CRATE)?;
        runtime.push(library);
    }
    let library = release.join(prebuilt::dylib_name("sunrise_patch"));
    super::library_record::validate_exports(&library, "sunrise_patch")?;
    super::library_record::stamp(
        &library,
        "sunrise_patch",
        identities.recorded("sunrise_patch"),
        true,
    )?;
    runtime.push(library);
    for library in outputs.runtime(
        &metadata.runtime_packages()?,
        target_dir,
        &rust.cargo_home(),
    )? {
        if let Some(existing) = runtime
            .iter()
            .find(|existing| existing.file_name() == library.file_name())
        {
            if fs::read(existing).map_err(|e| e.to_string())?
                != fs::read(&library).map_err(|e| e.to_string())?
            {
                return Err(format!(
                    "Native runtime output {} conflicts with an immutable SDK library",
                    library.display()
                ));
            }
        } else {
            runtime.push(library);
        }
    }
    Ok(Some((binary, runtime)))
}

/// Gives each file under `staged` whose bytes equal the file at the same place under `previous`
/// that file's modified time, and each folder whose entries have the same names as the folder at
/// the same place that folder's: a build script whose `rerun-if-changed` names a folder reruns
/// when anything in it, the folder itself included, is newer than its last run.
fn keep_unchanged_times(staged: &Path, previous: &Path) -> std::io::Result<()> {
    let mut names = Vec::new();
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let old = previous.join(entry.file_name());
        names.push(entry.file_name());
        if kind.is_dir() {
            if old.is_dir() {
                keep_unchanged_times(&entry.path(), &old)?;
            }
        } else if kind.is_file() {
            let Ok(meta) = fs::symlink_metadata(&old) else {
                continue;
            };
            if meta.is_file()
                && meta.len() == entry.metadata()?.len()
                && fs::read(&old)? == fs::read(entry.path())?
            {
                fs::File::options()
                    .write(true)
                    .open(entry.path())?
                    .set_modified(meta.modified()?)?;
            }
        }
    }
    let mut old_names = fs::read_dir(previous)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    old_names.sort();
    // Setting a time inside a folder leaves the folder's own time alone, so this comes last.
    if names == old_names {
        filetime::set_file_mtime(
            staged,
            filetime::FileTime::from_last_modification_time(&fs::metadata(previous)?),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// A restaged file whose bytes didn't change keeps its old modified time, so cargo reuses
    /// what it built from it; a changed or new file keeps its new one.
    #[test]
    fn only_unchanged_staged_files_keep_their_old_time() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("stage.old"), root.path().join("stage"));
        for dir in [&old, &new] {
            fs::create_dir_all(dir.join("src")).unwrap();
        }
        let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        for (name, text) in [("src/lib.rs", "same"), ("Cargo.toml", "old")] {
            let path = old.join(name);
            fs::write(&path, text).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(then)
                .unwrap();
        }
        fs::write(new.join("src/lib.rs"), "same").unwrap();
        fs::write(new.join("Cargo.toml"), "new").unwrap();
        fs::write(new.join("src/added.rs"), "added").unwrap();
        keep_unchanged_times(&new, &old).unwrap();
        let modified = |name: &str| fs::metadata(new.join(name)).unwrap().modified().unwrap();
        assert_eq!(modified("src/lib.rs"), then);
        assert_ne!(modified("Cargo.toml"), then);
        assert_ne!(modified("src/added.rs"), then);
    }

    /// A restaged folder whose entries didn't change keeps its old modified time; one that gained
    /// an entry keeps its new one.
    #[test]
    fn only_folders_with_the_same_entries_keep_their_old_time() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("stage.old"), root.path().join("stage"));
        for dir in [&old, &new] {
            for folder in ["data/inner", "grown"] {
                fs::create_dir_all(dir.join(folder)).unwrap();
            }
            fs::write(dir.join("data/inner/a.bin"), "a").unwrap();
        }
        fs::write(new.join("grown/added.rs"), "added").unwrap();
        let then = filetime::FileTime::from_unix_time(1_000_000, 0);
        for folder in ["data/inner", "data", "grown"] {
            filetime::set_file_mtime(old.join(folder), then).unwrap();
        }
        keep_unchanged_times(&new, &old).unwrap();
        let modified = |name: &str| {
            filetime::FileTime::from_last_modification_time(&fs::metadata(new.join(name)).unwrap())
        };
        assert_eq!(modified("data"), then);
        assert_eq!(modified("data/inner"), then);
        assert_ne!(modified("grown"), then);
    }

    /// Restaged twice with nothing changed, a build script whose `rerun-if-changed` names a folder
    /// runs only in the first build: cargo finds nothing in the folder newer than that run.
    #[test]
    #[ignore = "daily"]
    fn a_build_script_watching_a_restaged_folder_stays_fresh() {
        let root = tempfile::tempdir().unwrap();
        let (stage, previous) = (root.path().join("stage"), root.path().join("stage.old"));
        let runs = root.path().join("runs");
        let write_stage = || {
            fs::create_dir_all(stage.join("src/data")).unwrap();
            fs::write(
                stage.join("Cargo.toml"),
                "[package]\nname = \"watcher\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n",
            )
            .unwrap();
            fs::write(stage.join("src/lib.rs"), "").unwrap();
            fs::write(stage.join("src/data/table.txt"), "rows").unwrap();
            fs::write(
                stage.join("build.rs"),
                format!(
                    "fn main() {{\n    println!(\"cargo::rerun-if-changed=src/data\");\n    \
                     let mut runs = std::fs::OpenOptions::new().create(true).append(true).open({:?}).unwrap();\n    \
                     std::io::Write::write_all(&mut runs, b\"run\\n\").unwrap();\n}}\n",
                    runs
                ),
            )
            .unwrap();
        };
        let build = || {
            let status =
                std::process::Command::new(std::env::var_os("CARGO").unwrap_or("cargo".into()))
                    .args(["build", "--offline", "--quiet", "--target-dir"])
                    .arg(root.path().join("target"))
                    .current_dir(&stage)
                    .status()
                    .unwrap();
            assert!(status.success());
        };
        write_stage();
        build();
        for _ in 0..2 {
            // A second's gap, so a folder written again is newer than the last run even where
            // the filesystem keeps whole seconds.
            std::thread::sleep(std::time::Duration::from_secs(1));
            fs::rename(&stage, &previous).unwrap();
            write_stage();
            keep_unchanged_times(&stage, &previous).unwrap();
            fs::remove_dir_all(&previous).unwrap();
            build();
        }
        assert_eq!(fs::read_to_string(&runs).unwrap(), "run\n");
    }

    use super::*;

    #[test]
    fn generated_facade_cannot_replace_an_authored_registry_alias() {
        let dir = tempfile::tempdir().unwrap();
        super::super::tests::write_bundle(dir.path(), "bake", "");
        let bundle = super::super::bundle_in(dir.path()).unwrap().unwrap();
        for (sdk_key, expected_alias) in [
            ("sunrise-internals", None),
            ("game-internals", Some("game_internals")),
        ] {
            let cargo = format!(
                r#"
                [package]
                name = "bake"
                version = "0.1.0"
                [dependencies]
                sunrise-mod-api = {{ path = ".sunrise/sdk/api/crates/sunrise-mod-api" }}
                {sdk_key} = {{ package = "sunrise-internals", path = ".sunrise/sdk/api/crates/sunrise-internals" }}
                internals = {{ package = "itoa", version = "1" }}
                "#
            );
            let project = api::Project::parse(&bundle, &cargo).unwrap();
            let (mut manifest, wanted) =
                super::super::crates::rewrite_closure(&project, dir.path(), |_, _| {
                    unreachable!("fixture has no local dependencies")
                })
                .unwrap();
            let result = add_facade_dependencies(
                &project,
                &mut manifest,
                "native-internals/bake",
                "sunrise-internals-bake",
            );
            let dependencies = &manifest["dependencies"];
            assert_eq!(dependencies["internals"]["package"].as_str(), Some("itoa"));
            assert_eq!(dependencies["internals"]["version"].as_str(), Some("1"));
            assert_eq!(wanted.len(), 1);
            assert_eq!(wanted[0].name, "itoa");
            if let Some(alias) = expected_alias {
                result.unwrap();
                assert_eq!(
                    dependencies[alias]["package"].as_str(),
                    Some("sunrise-internals-bake")
                );
                assert_eq!(
                    dependencies[alias]["path"].as_str(),
                    Some("../../native-internals/bake")
                );
            } else {
                let error = result.unwrap_err();
                assert!(
                    error.contains("bake: Cargo.toml dependency alias `internals`"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn generated_facade_alias_collides_with_a_hyphenated_registry_alias() {
        let dir = tempfile::tempdir().unwrap();
        super::super::tests::write_bundle(dir.path(), "bake", "");
        let bundle = super::super::bundle_in(dir.path()).unwrap().unwrap();
        let cargo = r#"
            [package]
            name = "bake"
            version = "0.1.0"
            [dependencies]
            sunrise-mod-api = { path = ".sunrise/sdk/api/crates/sunrise-mod-api" }
            game-internals = { package = "itoa", version = "1" }
            game_internals = { package = "sunrise-internals", path = ".sunrise/sdk/api/crates/sunrise-internals" }
            "#;
        let project = api::Project::parse(&bundle, cargo).unwrap();
        let (mut manifest, _) =
            super::super::crates::rewrite_closure(&project, dir.path(), |_, _| {
                unreachable!("fixture has no local dependencies")
            })
            .unwrap();
        let error = add_facade_dependencies(
            &project,
            &mut manifest,
            "native-internals/bake",
            "sunrise-internals-bake",
        )
        .unwrap_err();
        assert!(
            error.contains("bake: Cargo.toml dependency alias `"),
            "{error}"
        );
        assert_eq!(
            manifest["dependencies"]["game-internals"]["package"].as_str(),
            Some("itoa")
        );
    }

    #[test]
    fn registry_staging_inserts_the_implicit_library_root() {
        let stage = tempfile::tempdir().unwrap();
        let src = stage.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("lib.rs"), "pub fn library() {}\n").unwrap();
        let mut manifest = toml::toml! { [package] name = "example" version = "1.0.0" };

        let root = stage_library(stage.path(), &mut manifest, "example", "// generated\n").unwrap();
        let configured = manifest["lib"]["path"].as_str().unwrap();
        assert_eq!(stage.path().join(configured), root);
        assert_eq!(root, src.join("lib.rs"));
        assert!(
            fs::read_to_string(root)
                .unwrap()
                .contains("mod __sunrise_entry;")
        );
        assert_eq!(
            fs::read_to_string(src.join("__sunrise_entry.rs")).unwrap(),
            "// generated\n"
        );
    }

    #[test]
    fn forwarding_sdk_requires_one_logger_slot_in_its_template() {
        assert!(check_log_template("fn entry() {}", false).is_ok());
        assert!(check_log_template("fn entry() {}", true).is_err());
        assert!(check_log_template("/* MOD_LOGGERS */", true).is_ok());
        assert!(check_log_template("/* MOD_LOGGERS */ /* MOD_LOGGERS */", true).is_err());
    }

    #[test]
    #[ignore = "daily"]
    fn registry_adapter_compiles_facade_with_transitive_host_macro_metadata() {
        use std::{ffi::OsString, process::Command};
        let target =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/registry-facade-tests");
        fs::create_dir_all(&target).unwrap();
        let scratch = tempfile::tempdir_in(target).unwrap();
        let game = scratch.path().join("sdk");
        for folder in ["deps", "host", "native"] {
            fs::create_dir_all(game.join(folder)).unwrap();
        }
        let sysroot = Command::new("rustc")
            .arg("--print")
            .arg("sysroot")
            .output()
            .unwrap();
        assert!(sysroot.status.success());
        let rust = crate::rust::tests::system(PathBuf::from(
            String::from_utf8(sysroot.stdout).unwrap().trim(),
        ));
        let macros = game.join("host").join(format!(
            "{}sdk_macros{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
        let macro_source = scratch.path().join("macros.rs");
        fs::write(&macro_source, "extern crate proc_macro; #[proc_macro_derive(Marker)] pub fn marker(_: proc_macro::TokenStream) -> proc_macro::TokenStream { proc_macro::TokenStream::new() }").unwrap();
        let built = Command::new(rust.rustc())
            .args([
                "--crate-name",
                "sdk_macros",
                "--crate-type",
                "proc-macro",
                "--edition",
                "2024",
            ])
            .arg(&macro_source)
            .arg("-o")
            .arg(&macros)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let engine = game.join("deps/libsunrise_client-fixture.rmeta");
        let engine_source = scratch.path().join("engine.rs");
        fs::write(
            &engine_source,
            "pub use sdk_macros::Marker; #[derive(Marker)] pub struct Game;",
        )
        .unwrap();
        let built = Command::new(rust.rustc())
            .args([
                "--crate-name",
                "sunrise_client",
                "--crate-type",
                "rlib",
                "--edition",
                "2024",
                "--emit",
                "metadata",
                "-C",
                "prefer-dynamic",
                "--target",
                rust.target(),
            ])
            .arg(&engine_source)
            .arg("--extern")
            .arg(format!("sdk_macros={}", macros.display()))
            .arg("-o")
            .arg(&engine)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let facade = scratch.path().join("facade");
        fs::create_dir_all(&facade).unwrap();
        let source = facade.join("lib.rs");
        fs::write(&source, "pub use sunrise_client::Game;").unwrap();
        let mut plan = compiler_plan(
            &rust,
            &game,
            BTreeMap::from([(
                facade.clone(),
                registry_rustc::Bindings {
                    externs: BTreeMap::from([("sunrise_client".into(), engine)]),
                    link_args: vec![],
                },
            )]),
        );
        plan.cache_wrapper = None;
        plan.workspace_wrapper = None;
        let args: Vec<OsString> = vec![
            rust.rustc().into_os_string(),
            source.into_os_string(),
            "--crate-name".into(),
            "facade".into(),
            "--crate-type".into(),
            "rlib".into(),
            "--edition".into(),
            "2024".into(),
            "--target".into(),
            rust.target().into(),
            "--emit".into(),
            "metadata".into(),
            "-o".into(),
            scratch.path().join("facade.rmeta").into_os_string(),
        ];
        let (program, arguments) = plan
            .arguments(args.clone(), Some(facade.clone()), scratch.path())
            .unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        // Render a real SDK-route executable workspace. Its SDK libraries are
        // binary externs, not Cargo source dependencies, including the host that
        // the SDK-owned template calls after the layer split.
        for name in [
            "mod_api",
            "mod_vocab",
            "sunrise_mod_package",
            "sunrise_mod_host",
        ] {
            let source = scratch.path().join(format!("{name}.rs"));
            fs::write(
                &source,
                if name == "sunrise_mod_host" {
                    "pub fn main_with() {}"
                } else {
                    ""
                },
            )
            .unwrap();
            let result = Command::new(rust.rustc())
                .args([
                    "--crate-name",
                    name,
                    "--crate-type",
                    "rlib",
                    "--edition",
                    "2024",
                    "--emit",
                    "metadata",
                    "--target",
                    rust.target(),
                ])
                .arg(source)
                .arg("-o")
                .arg(game.join(format!("deps/lib{name}-fixture.rmeta")))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        fs::write(
            game.join(prebuilt::TEMPLATE),
            "fn main() { sunrise_mod_host::main_with(); }\n/* MOD_ENTRIES */\n/* BUILT_MODS */\n",
        )
        .unwrap();
        let template = api::template(&game.join(prebuilt::TEMPLATE)).unwrap();
        let wrapper = scratch.path().join("wrapper");
        fs::create_dir_all(wrapper.join("src")).unwrap();
        fs::write(
            wrapper.join("Cargo.toml"),
            "[package]\nname='registry-game'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
        )
        .unwrap();
        fs::write(wrapper.join("src/main.rs"), api::main_rs(&template, &[])).unwrap();
        let bindings = prebuilt::wrapper_externs(&game.join("deps")).unwrap();
        let mut wrapper_plan = compiler_plan(
            &rust,
            &game,
            BTreeMap::from([(
                wrapper.clone(),
                registry_rustc::Bindings {
                    externs: bindings,
                    link_args: vec![],
                },
            )]),
        );
        wrapper_plan.cache_wrapper = None;
        wrapper_plan.workspace_wrapper = None;
        let mut wrapper_args = args.clone();
        wrapper_args[1] = wrapper.join("src/main.rs").into_os_string();
        wrapper_args[3] = "registry_game".into();
        wrapper_args[5] = "bin".into();
        *wrapper_args.last_mut().unwrap() = scratch.path().join("wrapper.rmeta").into_os_string();
        let (program, arguments) = wrapper_plan
            .arguments(wrapper_args.clone(), Some(wrapper.clone()), scratch.path())
            .unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        wrapper_plan
            .crates
            .get_mut(&wrapper)
            .unwrap()
            .externs
            .remove("sunrise_mod_host");
        let (program, arguments) = wrapper_plan
            .arguments(wrapper_args, Some(wrapper), scratch.path())
            .unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("E0433"));
        // The old registry route supplied deps alone, unlike the direct SDK route.
        plan.search = vec![game.join("deps")];
        let (program, arguments) = plan.arguments(args, Some(facade), scratch.path()).unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("E0463"));
    }

    #[test]
    #[ignore = "requires an explicitly prepared private SDK, mod and launcher executable"]
    fn real_sdk_registry_compile() {
        let required =
            |name| PathBuf::from(std::env::var_os(name).expect("explicit fixture path required"));
        let game = required("SUNRISE_TEST_REGISTRY_SDK");
        let folder = required("SUNRISE_TEST_REGISTRY_MOD");
        let out = required("SUNRISE_TEST_REGISTRY_OUT");
        let adapter = required("SUNRISE_TEST_REGISTRY_ADAPTER");
        let bundle = super::super::bundle_in(&folder).unwrap().unwrap();
        let projects = [api::Project::read(&bundle).unwrap()];
        let declared = [Vec::new()];
        let inputs = [dependencies::Mod {
            id: &bundle.manifest.id,
            version: &bundle.manifest.version,
            dependencies: &declared[0],
            code: true,
        }];
        let resolution = dependencies::resolve(&inputs).unwrap();
        let rust = crate::rust::tests::system(required("SUNRISE_TEST_REGISTRY_RUST"));
        let approve = std::env::var_os("SUNRISE_TEST_REGISTRY_DECLINE").is_none();
        let sdk = prebuilt::identity(&game).unwrap();
        let catalog = internals::Catalog::read(&game, sdk.commit()).unwrap();
        let result = stage_and_cargo(
            &rust,
            &game,
            &out,
            &out.join("target"),
            &projects,
            &declared,
            &resolution,
            Options {
                consent: Some(&|inventory| {
                    eprintln!("Registry dependencies: {inventory:?}");
                    approve
                }),
                adapter: &adapter,
                mixins: &crate::MixinsIdentity::resolve([&bundle], None).unwrap(),
                sdk: &sdk,
                catalog: &catalog,
            },
            false,
        );
        if approve {
            let (binary, _) = result.unwrap().unwrap();
            assert!(binary.is_file());
            eprintln!("compiled wrapper {}", binary.display());
        } else {
            assert!(result.unwrap_err().contains("\"registry\": true"));
            assert!(!out.join("cargo-messages.jsonl").exists());
            assert!(!out.join("target").exists());
            eprintln!(
                "detailed consent declined before target compilation or Cargo build messages"
            );
        }
    }
}
