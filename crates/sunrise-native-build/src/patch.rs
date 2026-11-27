//! Function replacement (fn-replace): `[[native.mixin]]` rows built against the prebuilt game, with no
//! game rebuild. Each function the rows target is copied out of the pinned source into one patch
//! crate (`sunrise_mod_api_gen::patch`), horizon-mixins applies every mod's rows to those copies
//! in one pass (one combined copy per function, in the tool's order), rustc compiles the crate
//! against the prebuilt game and the mods, and the generated `main` installs each copy through
//! its point's replacement slot. What can't take this route is refused by name; nothing here
//! falls back to compiling the game.
use super::{Bundle, MixinsIdentity, api, game_uses, prebuilt::Identity};
use crate::host::trf;
use crate::rust::Toolchain;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use sunrise_platform::mod_build::api_gen;

/// The patch crates' name (a lower game crate's is `<name>_<crate>`): the copies' ids and the library entry's
/// `--extern` use it. It is not `sunrise_patch`, the name of the library built from them: a library and a crate
/// it links with one crate name are indistinguishable to rustc (E0519), and a registry workspace refuses two
/// packages of one name.
pub const CRATE: &str = "sunrise_patched";

/// The compiled patch crates, `(crate name, rlib)`, and the installation result expressions the
/// generated `main` installs.
pub struct Patch {
    pub rlibs: Vec<(String, PathBuf)>,
    pub installs: Vec<String>,
    /// The mods whose libraries the patched crates link.
    pub mods: Vec<String>,
}

fn install_expression(target: &str, mods: &[&str], install: &str) -> String {
    format!("({target:?}, &{mods:?}, {install})")
}

/// Preserve every requester at the launcher boundary; the generator only knows targets.
fn target_refusal(
    refused: &api_gen::patch::Refusal,
    requests: &[(String, String, String)],
) -> String {
    let owners = requests
        .iter()
        .filter(|(_, _, target)| target == &refused.target)
        .map(|(id, row, _)| format!("{id}:{row}"))
        .collect::<Vec<_>>()
        .join(", ");
    if owners.is_empty() {
        // Module and bridge ids already include their owning mod.
        return refused.to_string();
    }
    trf(
        "{code}: {owners}: {target}: {why}",
        &[
            ("code", refused.code),
            ("owners", &owners),
            ("target", &refused.target),
            ("why", &refused.why),
        ],
    )
}

/// The support modules and bridges `bundles` add to the patch crate.
fn extras(bundles: &[&Bundle]) -> Result<api_gen::patch::Extras, String> {
    let mut extras = api_gen::patch::Extras::default();
    // The names the helpers and bindings use, which a lower crate's mirror keeps imports for.
    for bundle in bundles {
        for row in &bundle.native().mixin {
            let mut texts =
                vec![fs::read_to_string(bundle.dir.join(&row.helper)).unwrap_or_default()];
            // `bindings` and every other forwarded key: their strings name types and values.
            fn strings(value: &toml::Value, out: &mut Vec<String>) {
                match value {
                    toml::Value::String(text) => out.push(text.replace('$', " ")),
                    toml::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
                    toml::Value::Table(table) => table.values().for_each(|v| strings(v, out)),
                    _ => {}
                }
            }
            for value in row.options.values() {
                strings(value, &mut texts);
            }
            for text in texts {
                if let Ok(tokens) = text.parse::<proc_macro2::TokenStream>() {
                    api_gen::patch::idents_in(&tokens, &mut extras.idents);
                }
            }
        }
    }
    let normalize = |s: &str| s.replace('-', "_").to_ascii_lowercase();
    for bundle in bundles {
        let id = &bundle.manifest.id;
        let Some(native) = bundle.manifest.native.as_ref() else {
            continue;
        };
        for module in &native.module {
            let path = bundle.dir.join(&module.source);
            extras.modules.push(api_gen::patch::SupportModule {
                id: format!("{id}:{}", module.id),
                file: module.file.clone(),
                name: module.name.clone(),
                text: fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?,
                public: module.visibility.as_deref() == Some("pub"),
            });
        }
        for bridge in &native.bridge {
            let field = |key: &str| bridge.fields.get(key).and_then(toml::Value::as_str);
            extras.bridges.push(api_gen::patch::Bridge {
                id: format!("{id}:{}", bridge.id),
                method: format!(
                    "__horizon_mixins_{}_{}",
                    normalize(id),
                    normalize(&bridge.id)
                ),
                kind: field("kind").unwrap_or_default().to_owned(),
                target: field("target").unwrap_or_default().to_owned(),
                member: field("member").map(str::to_owned),
            });
        }
    }
    Ok(extras)
}

/// The patched sources of `projects`' `[[native.mixin]]` rows, supports modules and bridges,
/// ready to compile: one crate folder per name under `dir`.
pub struct Prepared<'a> {
    pub dir: PathBuf,
    pub names: Vec<String>,
    pub(crate) owners: BTreeMap<String, String>,
    pub installs: Vec<String>,
    /// The mods whose helpers the crates may call.
    pub bundles: Vec<&'a Bundle>,
    count: usize,
    rows: usize,
    started: std::time::Instant,
    copied: std::time::Duration,
    applied: std::time::Duration,
}

/// Builds the patch crates for `projects`' `[[native.mixin]]` rows against `game`, into `out`;
/// `None` when no mod has one. `rlibs` and `crates` are the compiled mods and their crate names,
/// which helpers may call.
#[allow(clippy::too_many_arguments)]
pub fn build(
    rust: &dyn Toolchain,
    game: &Path,
    out: &Path,
    projects: &[api::Project<'_>],
    rlibs: &HashMap<&str, PathBuf>,
    crates: &HashMap<&str, String>,
    sdk: &Identity,
    mixins: &MixinsIdentity,
    rustc: impl Fn(&str) -> std::process::Command,
    check_only: bool,
) -> Result<Option<Patch>, String> {
    let Some(prepared) = prepare(rust, game, out, projects, sdk, mixins, check_only)? else {
        return Ok(None);
    };
    // The patched crates, against the prebuilt game and the mods.
    let mut compiled = Vec::new();
    for name in &prepared.names {
        let owner = &prepared.owners[name];
        let features = sdk.patch_features(owner)?;
        let externs = sdk_externs(game, sdk, owner)?;
        let extension = if check_only { "rmeta" } else { "rlib" };
        let rlib = out.join(format!("lib{name}.{extension}"));
        let mut hash = sha2::Sha256::new();
        use sha2::Digest;
        hash.update(sdk.commit());
        hash.update(format!("{owner}:{features:?}:{externs:?}"));
        for bundle in &prepared.bundles {
            hash.update(
                super::source_digest(bundle)
                    .ok_or(crate::host::tr("Could not hash patch inputs"))?,
            );
            hash.update(fs::read(&rlibs[bundle.manifest.id.as_str()]).map_err(|e| e.to_string())?);
        }
        let fingerprint = format!("{:x}", hash.finalize());
        let saved = rlib.with_extension("fingerprint");
        if rlib.is_file() && fs::read_to_string(&saved).ok().as_deref() == Some(&fingerprint) {
            compiled.push((name.clone(), rlib));
            continue;
        }
        let mut cmd = rustc("2024");
        cmd.args([
            "--crate-type",
            "rlib",
            "--crate-name",
            name,
            "--cap-lints",
            "allow",
        ])
        .arg("-o")
        .arg(&rlib)
        .arg(prepared.dir.join(name).join("src/lib.rs"));
        if check_only {
            cmd.arg("--emit=metadata");
        }
        game_cfg(&mut cmd, features);
        // An `@` file, one argument per line: an extern per SDK crate passes Windows'
        // 32,767-character command-line limit.
        let mut args = Vec::new();
        for (krate, library) in externs {
            args.push(format!("--extern\n{krate}={}", library.display()));
        }
        for bundle in &prepared.bundles {
            let id = bundle.manifest.id.as_str();
            args.push(format!("--extern\n{}={}", crates[id], rlibs[id].display()));
        }
        let argfile = out.join(format!("{name}.externs"));
        fs::write(&argfile, args.join("\n"))
            .map_err(|e| format!("Could not write {}: {e}", argfile.display()))?;
        let mut arg = std::ffi::OsString::from("@");
        arg.push(&argfile);
        cmd.arg(arg);
        super::run(cmd, "Compiling the patched functions")?;
        fs::write(saved, fingerprint).map_err(|e| e.to_string())?;
        compiled.push((name.clone(), rlib));
    }
    prepared.log()?;
    Ok(Some(Patch {
        rlibs: compiled,
        mods: prepared
            .bundles
            .iter()
            .map(|bundle| bundle.manifest.id.clone())
            .collect(),
        installs: prepared.installs,
    }))
}

fn sdk_extern_names(sdk: &Identity) -> BTreeSet<&str> {
    let mut names: BTreeSet<_> = sdk.facade_crates().collect();
    names.insert("sunrise_client");
    if let Some(role) = sdk.runtime_role() {
        names.extend(
            role.libraries
                .values()
                .filter(|fact| {
                    matches!(
                        fact.package.as_str(),
                        "sunrise-gameplay-common" | "sunrise-gameplay" | "sunrise-client-gameplay"
                    )
                })
                .map(|fact| fact.library.as_str()),
        );
    }
    names
}

pub(crate) fn sdk_externs(
    game: &Path,
    sdk: &Identity,
    owner: &str,
) -> Result<BTreeMap<String, PathBuf>, String> {
    let deps = game.join("deps");
    let role = sdk.runtime_role();
    let mut externs = BTreeMap::new();
    for name in sdk_extern_names(sdk) {
        let role_library = role.and_then(|role| {
            let mut libraries = role.libraries.values().filter(|fact| fact.library == name);
            let library = libraries.next()?;
            libraries.next().is_none().then_some(library)
        });
        let path = if let Some(fact) = role_library {
            super::prebuilt::role_library(&deps, fact)?
        } else if name == "sunrise_client" || role.is_some() {
            super::prebuilt::library(&deps, name)?
        } else {
            super::prebuilt::facade_library(&deps, name, sdk)?
        };
        externs.insert(name.to_owned(), path);
    }
    if let (Some(role), Some(fact)) = (role, sdk.patch_library(owner)?) {
        for (alias, identity) in &fact.aliases {
            let dependency = &role.libraries[identity];
            let path = super::prebuilt::role_library(&deps, dependency)?;
            externs.insert(alias.clone(), path);
        }
    }
    Ok(externs)
}

impl Prepared<'_> {
    /// The build log's line for this mixin build.
    pub fn log(&self) -> Result<(), String> {
        super::append_log(&trf(
            "Function replacement: {count} functions patched by {rows} mixins, without rebuilding the game (copy {copy} ms, horizon-mixins {tool} ms, compile {compile} ms, total {total} ms)",
            &[
                ("count", &self.count.to_string()),
                ("rows", &self.rows.to_string()),
                ("copy", &self.copied.as_millis().to_string()),
                (
                    "tool",
                    &(self.applied - self.copied).as_millis().to_string(),
                ),
                (
                    "compile",
                    &(self.started.elapsed() - self.applied)
                        .as_millis()
                        .to_string(),
                ),
                ("total", &self.started.elapsed().as_millis().to_string()),
            ],
        ))
    }
}

/// The published loops `bundles`' `[[native.loop]]` rows fill, with their slots in the order they
/// are asked, from the game's `loops.json`. A row naming no published loop, and a mixin that
/// replaces or redirects a filled loop's entry (anything but `HEAD` and `RETURN`, which compose
/// around the loop), are refused by name.
fn loop_slots(
    game: &Path,
    commit: &str,
    bundles: &[&Bundle],
    index: &api_gen::functions::FunctionIndex,
) -> Result<Vec<api_gen::patch::LoopSlots>, String> {
    use sunrise_mod_package::manifest::mixin::Kind;
    let published =
        game_uses::published_loops(game, commit).map_err(|e| format!("E_MIXIN_INDEX: {e}"))?;
    let mut ordered: Vec<_> = bundles
        .iter()
        .flat_map(|bundle| bundle.native().r#loop.iter().map(move |row| (*bundle, row)))
        .collect();
    let mut refusals = Vec::new();
    // A loop with a mixin on its function is copied with it, slots or none.
    let mut filled: Vec<api_gen::patch::LoopSlots> = published
        .entries
        .iter()
        .filter(|l| {
            let generic = l.generic_target();
            bundles.iter().any(|b| {
                b.native()
                    .mixin
                    .iter()
                    .any(|m| index.target_key(&m.target) == index.target_key(&generic))
            })
        })
        .map(|l| api_gen::patch::LoopSlots {
            published: l.clone(),
            slots: Vec::new(),
        })
        .collect();
    ordered.sort_by(|(a, ra), (b, rb)| {
        (ra.priority, &a.manifest.id, &ra.id).cmp(&(rb.priority, &b.manifest.id, &rb.id))
    });
    for (bundle, row) in ordered {
        let id = &bundle.manifest.id;
        let found = match published.require(id, row) {
            Ok(found) => found,
            Err(refusal) => {
                refusals.push(refusal);
                continue;
            }
        };
        let path = bundle.dir.join(&row.helper);
        let helper = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let slot = api_gen::patch::LoopSlot {
            mod_id: id.clone(),
            id: row.id.clone(),
            helper,
            with: row.with.clone(),
        };
        match filled.iter_mut().find(|l| l.published.name == found.name) {
            Some(slots) => slots.slots.push(slot),
            None => filled.push(api_gen::patch::LoopSlots {
                published: found.clone(),
                slots: vec![slot],
            }),
        }
    }
    for bundle in bundles {
        for mixin in &bundle.native().mixin {
            let Some(slots) = filled.iter().find(|l| {
                !l.slots.is_empty()
                    && (index.target_key(&l.published.entry) == index.target_key(&mixin.target)
                        || (index.target_key(&l.published.generic_target())
                            == index.target_key(&mixin.target)
                            && mixin.kind == Kind::Overwrite))
            }) else {
                continue;
            };
            if mixin.kind == Kind::Overwrite
                || !matches!(mixin.kind, Kind::Head | Kind::Return)
                    && index.target_key(&slots.published.entry) == index.target_key(&mixin.target)
            {
                let fillers: Vec<&str> = slots.slots.iter().map(|s| s.mod_id.as_str()).collect();
                refusals.push(format!(
                    "E_LOOP_ENTRY_REPLACED: {}:{}: the {} mixin on {} would replace or bypass the published loop `{}`, filled by {}; only HEAD and RETURN mixins compose with it",
                    bundle.manifest.id,
                    mixin.id,
                    mixin.kind.name(),
                    mixin.target,
                    slots.published.name,
                    fillers.join(", ")
                ));
            }
        }
    }
    if refusals.is_empty() {
        Ok(filled)
    } else {
        Err(refusals.join("\n"))
    }
}

/// Copies, generates and transforms the patch crates for `projects` into `out`, without
/// compiling them; `None` when no mod has a mixin, module, bridge or loop.
pub fn prepare<'a>(
    rust: &dyn Toolchain,
    game: &Path,
    out: &Path,
    projects: &[api::Project<'a>],
    sdk: &Identity,
    mixins: &MixinsIdentity,
    check_only: bool,
) -> Result<Option<Prepared<'a>>, String> {
    let bundles: Vec<&Bundle> = projects
        .iter()
        .map(|project| project.bundle)
        .filter(|bundle| {
            bundle.needs_mixins()
                || bundle
                    .manifest
                    .native
                    .as_ref()
                    .is_some_and(|n| !n.r#loop.is_empty())
        })
        .collect();
    if bundles.is_empty() {
        return Ok(None);
    }
    let index =
        game_uses::function_index(game, sdk.commit()).map_err(|e| format!("E_MIXIN_INDEX: {e}"))?;
    let loops = loop_slots(game, sdk.commit(), &bundles, &index)?;
    let requests: Vec<(String, String, String)> = bundles
        .iter()
        .flat_map(|bundle| {
            bundle.native().mixin.iter().map(|row| {
                (
                    bundle.manifest.id.clone(),
                    row.id.clone(),
                    row.target.clone(),
                )
            })
        })
        .collect();
    let rows: Vec<String> = bundles
        .iter()
        .flat_map(|bundle| bundle.native().mixin.iter().map(|row| row.target.clone()))
        .collect();
    // Built once per pin and patch, before the clock: the timing is the mixin build's own.
    let tool = if rows.is_empty() {
        None
    } else {
        Some(super::mixins_patch::tool(rust, mixins)?)
    };
    let source = super::source(sdk.commit())?;
    let started = std::time::Instant::now();
    // A filled loop's entry, or one whose loop function has rows, is copied like a mixin's target,
    // with the loop's body.
    let targets: Vec<String> = rows
        .iter()
        .cloned()
        .chain(loops.iter().map(|l| l.published.entry.clone()))
        .collect();
    let loop_count = loops.iter().filter(|l| !l.slots.is_empty()).count();
    let slot_count = loops.iter().map(|l| l.slots.len()).sum::<usize>();
    // The generator refuses targets the staged game cannot replace.
    let mut extras = extras(&bundles)?;
    extras.loops = loops;
    extras.linked = sdk_extern_names(sdk)
        .into_iter()
        .map(str::to_owned)
        .collect();
    if let Some(role) = sdk.runtime_role() {
        extras.linked.extend(
            role.libraries
                .values()
                .flat_map(|fact| fact.aliases.keys().cloned()),
        );
    }
    extras.lower_crate_cfg = Some(match sdk.lower_crate_cfg() {
        None if sdk.runtime_role().is_some() => BTreeMap::new(),
        Some(recorded) => recorded.clone(),
        // A check only types the mods against the SDK, so an SDK that predates the lower
        // crates' records takes the pinned compiler's cfg for each of them, as `build` does
        // through the SDK's own tool.
        None if check_only => {
            let atoms = printed_cfg(rust, sdk.legacy_game_features()?)?;
            index
                .crates
                .keys()
                .map(|krate| (krate.clone(), atoms.clone()))
                .collect()
        }
        None => BTreeMap::new(),
    });
    if let Some(role) = sdk.runtime_role() {
        let target_cfg = printed_cfg(rust, &[])?;
        let cfg = extras.lower_crate_cfg.as_mut().expect("initialized above");
        for fact in role.libraries.values() {
            cfg.entry(fact.package.replace('-', "_"))
                .or_insert_with(|| {
                    let mut cfg = target_cfg.clone();
                    cfg.extend(
                        fact.features
                            .iter()
                            .map(|feature| format!("feature=\"{feature}\"")),
                    );
                    cfg
                });
        }
    }
    let generated = if sdk.runtime_role().is_some() {
        api_gen::patch::generate(&source, CRATE, &targets, &extras, &index)
    } else {
        api_gen::patch::generate_for_main_sdk(&source, CRATE, &targets, &extras, &index)
    };
    let refusals: Vec<String> = match &generated {
        Ok(_) => Vec::new(),
        Err(refused) => refused
            .iter()
            .map(|r| target_refusal(r, &requests))
            .collect(),
    };
    if !refusals.is_empty() {
        return Err(refusals.join("\n"));
    }
    let generated = generated.expect("no refusals");
    let ids: HashMap<String, &str> = generated
        .patched
        .iter()
        .map(|p| (index.target_key(&p.target), p.id.as_str()))
        .collect();
    let write = |path: PathBuf, text: &str| {
        fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let names: Vec<&str> = generated
        .libs
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    let groups: Vec<Vec<&str>> = if sdk.runtime_role().is_some() {
        names.iter().map(|name| vec![*name]).collect()
    } else {
        vec![names.clone()]
    };
    let scratch = out.join("patch-input");
    let patched = out.join("patch-applied");
    remove_previous_output(&scratch)?;
    remove_previous_output(&patched)?;
    fs::create_dir_all(&patched).map_err(|e| e.to_string())?;
    let mut tool_time = std::time::Duration::ZERO;
    for (number, group) in groups.iter().enumerate() {
        let input = scratch.join(number.to_string());
        let workspace = input.join("workspace");
        let mods = input.join("mods");
        let output = input.join("applied");
        let mut has_rows = false;
        for bundle in &bundles {
            let mut rows = bundle.native().mixin.clone();
            rows.retain(|row| {
                let id = ids[&index.target_key(&row.target)];
                id.split_once("::")
                    .is_some_and(|(name, _)| group.contains(&name))
            });
            if rows.is_empty() {
                continue;
            }
            has_rows = true;
            let dir = mods.join(&bundle.manifest.id);
            super::package::copy_tool_input(bundle, &dir)?;
            for row in &mut rows {
                row.target = ids[&index.target_key(&row.target)].to_owned();
            }
            let manifest = toml::toml! {
                [mod]
                id = (bundle.manifest.id.clone())
                version = (bundle.manifest.version.to_string())
                [compatibility]
                horizon_revision = (sdk.commit())
            };
            let mut manifest = manifest;
            manifest.insert(
                "injection".into(),
                toml::Value::try_from(&rows).map_err(|e| e.to_string())?,
            );
            write(
                dir.join("horizon-mixin.toml"),
                &toml::to_string(&manifest).map_err(|e| e.to_string())?,
            )?;
        }
        fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
        write(
            workspace.join("Cargo.toml"),
            &format!("[workspace]\nmembers = {group:?}\nresolver = \"2\"\n"),
        )?;
        write(
            workspace.join("Cargo.lock"),
            &format!(
                "# Generated by the launcher.\nversion = 4\n{}",
                group
                    .iter()
                    .map(|name| format!("\n[[package]]\nname = \"{name}\"\nversion = \"0.0.0\"\n"))
                    .collect::<String>()
            ),
        )?;
        write(workspace.join(".horizon-revision"), sdk.commit())?;
        for (name, lib) in &generated.libs {
            if !group.contains(&name.as_str()) {
                continue;
            }
            let package = workspace.join(name);
            fs::create_dir_all(package.join("src")).map_err(|e| e.to_string())?;
            write(
                package.join("Cargo.toml"),
                &format!(
                    "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.rs\"\n"
                ),
            )?;
            write(package.join("src/lib.rs"), lib)?;
        }
        let source = if has_rows {
            let applying = std::time::Instant::now();
            let tool = tool.as_ref().expect("rows have an apply tool");
            let name = if sdk.runtime_role().is_some() {
                group[0]
            } else {
                CRATE
            };
            let owner = generated
                .owners
                .get(name)
                .map_or("sunrise_client", String::as_str);
            let mut cmd = super::apply(rust, tool, &workspace, &mods, &output);
            for option in printed_cfg(rust, sdk.patch_features(owner)?)? {
                cmd.arg("--cfg").arg(option);
            }
            cmd.arg("--features-of").arg(name);
            super::run(cmd, "Applying the mixins to their functions")?;
            tool_time += applying.elapsed();
            output
        } else {
            workspace
        };
        for name in group {
            fs::rename(source.join(name), patched.join(name)).map_err(|e| e.to_string())?;
        }
    }
    let applied = started.elapsed();
    let copied = applied.saturating_sub(tool_time);
    if loop_count > 0 {
        super::append_log(&trf(
            "Published loops: {loops} filled by {slots} mod slots",
            &[
                ("loops", &loop_count.to_string()),
                ("slots", &slot_count.to_string()),
            ],
        ))?;
    }
    Ok(Some(Prepared {
        dir: patched,
        names: names.iter().map(|name| name.to_string()).collect(),
        owners: generated.owners,
        installs: generated
            .patched
            .iter()
            .filter_map(|p| {
                let mut mods = requests
                    .iter()
                    .filter(|(_, _, target)| {
                        index.target_key(target) == index.target_key(&p.target)
                    })
                    .map(|(id, _, _)| id.as_str())
                    .collect::<Vec<_>>();
                // Published loop copies carry their slot owners too.
                for l in &extras.loops {
                    if index.target_key(&l.published.entry) == index.target_key(&p.target) {
                        mods.extend(l.slots.iter().map(|slot| slot.mod_id.as_str()));
                    }
                }
                mods.sort_unstable();
                mods.dedup();
                Some(install_expression(&p.target, &mods, p.install.as_ref()?))
            })
            .collect(),
        bundles,
        count: generated.patched.len(),
        rows: rows.len(),
        started,
        copied,
        applied,
    }))
}

/// Removes the last build's output at `path`: a folder, a file, or a link in its place, never what
/// a link points at. A link is unlinked: a file symlink with `remove_file`, a directory symlink or
/// a Windows junction with `remove_dir`. Nothing there is fine; any other failure stops the build
/// with its own error. `path`'s folder is the build's own ([`super::prebuilt`] refuses a link
/// there), so nothing outside the build is reached through it.
fn remove_previous_output(path: &Path) -> Result<(), String> {
    let removed = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => unlink(path, meta.file_type()),
        Ok(meta) if meta.is_file() => fs::remove_file(path),
        Ok(_) => fs::remove_dir_all(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    };
    removed.map_err(|e| format!("{}: {e}", path.display()))
}

/// Unlinks the link at `path`: on Windows a directory symlink or junction with `remove_dir` and a
/// file symlink with `remove_file`; elsewhere every link with `remove_file`.
fn unlink(path: &Path, kind: fs::FileType) -> std::io::Result<()> {
    #[cfg(windows)]
    if std::os::windows::fs::FileTypeExt::is_symlink_dir(&kind) {
        return fs::remove_dir(path);
    }
    let _ = kind;
    fs::remove_file(path)
}

/// The game's features as the `--cfg` flags the patch library is compiled with. The compile and
/// the `--print cfg` that the tool plans against both take them from here, so they cannot drift.
fn game_cfg(cmd: &mut std::process::Command, features: &[String]) {
    for feature in features {
        cmd.arg("--cfg").arg(format!("feature=\"{feature}\""));
    }
}

/// The pinned compiler's `--print cfg` for the SDK's target under the game's features.
fn printed_cfg(rust: &dyn Toolchain, features: &[String]) -> Result<Vec<String>, String> {
    let mut print = super::prebuilt::rustc_with(rust, &[], "2024");
    game_cfg(&mut print, features);
    print.args(["--print", "cfg", "--target", rust.target()]);
    let printed = print.output().map_err(|e| e.to_string())?;
    if !printed.status.success() {
        return Err(String::from_utf8_lossy(&printed.stderr).into_owned());
    }
    Ok(build_cfg(&String::from_utf8_lossy(&printed.stdout)))
}

/// The cfg options of the build the patch library is compiled into, in rustc's `--cfg` spelling:
/// what `rustc --print cfg` printed, which carries the game's features and the target's facts.
/// The tool decides only the features and the target facts from it; flag-dependent options such as
/// `debug_assertions` stay undecided there.
fn build_cfg(printed: &str) -> Vec<String> {
    printed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod refusal_tests {
    /// A retained SDK that predates the lower crates' cfg records is checked against the pinned
    /// compiler's own cfg for its target and the game's features, so a lower-crate mixin target
    /// is not refused for an empty cfg. Needs a real rustc.
    #[test]
    #[ignore = "daily"]
    fn the_checks_lower_crate_cfg_is_the_pinned_compilers_for_the_sdks_features() {
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap();
        let sysroot = String::from_utf8(sysroot.stdout).unwrap();
        use crate::rust::Toolchain as _;
        let rust = crate::rust::tests::system(sysroot.trim().into());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("sunrise-prebuilt.json"),
            serde_json::json!({
                "package_format": sunrise_mod_package::sdk::PACKAGE_FORMAT,
                "commit": "0123456789abcdef0123456789abcdef01234567",
                "rustc": "release: 1",
                "side": "client",
                "target": rust.target(),
                "registry_lock": "version = 4\n",
                "registry_patches": {},
                "native_links": [],
                "facade_crates": [],
                "game_features": ["audio"],
                "slices": { "runtime": [], "api": [] }
            })
            .to_string(),
        )
        .unwrap();
        let sdk = crate::prebuilt::identity(dir.path()).unwrap();
        assert!(sdk.lower_crate_cfg().is_none());
        let printed = super::printed_cfg(&rust, sdk.legacy_game_features().unwrap()).unwrap();
        assert!(
            printed.contains(&"feature=\"audio\"".to_owned()),
            "{printed:?}"
        );
        assert!(
            printed.iter().any(|atom| atom.starts_with("target_os=")),
            "{printed:?}"
        );
    }

    /// A rebuild in the same folder starts with no applied-patch output, whether the last build
    /// left a folder, a file or a link there; a link's target is untouched.
    #[test]
    fn the_last_builds_applied_output_is_removed_and_a_links_target_kept() {
        use super::remove_previous_output;
        use std::fs;
        use std::path::Path;
        let build = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let applied = build.path().join("patch-applied");
        fs::create_dir_all(applied.join("crate/src")).unwrap();
        fs::write(applied.join("crate/src/lib.rs"), "stale").unwrap();
        remove_previous_output(&applied).unwrap();
        assert!(fs::symlink_metadata(&applied).is_err());
        remove_previous_output(&applied).unwrap();
        fs::write(&applied, "stale").unwrap();
        remove_previous_output(&applied).unwrap();
        assert!(fs::symlink_metadata(&applied).is_err());

        let target = outside.path().join("patch-applied");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep"), "kept").unwrap();
        let file = outside.path().join("file");
        fs::write(&file, "kept").unwrap();
        #[cfg(unix)]
        let (link_dir, link_file) = (
            |to: &Path, at: &Path| std::os::unix::fs::symlink(to, at),
            |to: &Path, at: &Path| std::os::unix::fs::symlink(to, at),
        );
        #[cfg(windows)]
        let (link_dir, link_file) = (
            |to: &Path, at: &Path| std::os::windows::fs::symlink_dir(to, at),
            |to: &Path, at: &Path| std::os::windows::fs::symlink_file(to, at),
        );
        // Windows grants symlinks only to developer mode or an administrator: a link it refuses
        // for that is skipped, and any other failure fails the test.
        let made = |link: std::io::Result<()>| match link {
            Ok(()) => true,
            Err(e) if cfg!(windows) && e.raw_os_error() == Some(1314) => false,
            Err(e) => panic!("{e}"),
        };
        if made(link_dir(&target, &applied)) {
            remove_previous_output(&applied).unwrap();
            assert!(fs::symlink_metadata(&applied).is_err());
        }
        if made(link_file(&file, &applied)) {
            remove_previous_output(&applied).unwrap();
            assert!(fs::symlink_metadata(&applied).is_err());
        }
        assert_eq!(fs::read_to_string(target.join("keep")).unwrap(), "kept");
        assert_eq!(fs::read_to_string(&file).unwrap(), "kept");
    }

    #[test]
    fn the_build_cfg_is_the_printed_set() {
        assert_eq!(
            super::build_cfg("unix\ntarget_os=\"linux\"\nfeature=\"audio\"\n\n"),
            ["unix", "target_os=\"linux\"", "feature=\"audio\""]
        );
    }

    #[test]
    fn a_missing_or_foreign_function_index_is_refused_by_name() {
        let game = tempfile::tempdir().unwrap();
        let commit = crate::tests::A;
        let read = || super::game_uses::function_index(game.path(), commit);
        let error = read().unwrap_err().to_string();
        assert!(
            error.contains("functions.json") && error.contains("fetch the SDK again"),
            "{error}"
        );
        let write = |commit, functions| {
            let index = serde_json::json!({
                "schema": 1, "build": {"commit": commit},
                "crates": {"sunrise_client": {
                    "library": "sunrise_client", "modules": {"sunrise_client::game": []},
                }},
                "functions": functions, "refused": {},
            });
            std::fs::write(game.path().join("functions.json"), index.to_string()).unwrap();
        };
        write(crate::tests::B, serde_json::json!({}));
        let error = read().unwrap_err().to_string();
        assert!(error.contains("functions.json"), "{error}");
        assert!(
            error.contains(crate::tests::B) && error.contains(commit),
            "{error}"
        );
        write(commit, serde_json::json!({}));
        assert!(read().unwrap().functions.is_empty());
        // A tampered index whose module the copy would write as Rust is refused, not a panic.
        write(
            commit,
            serde_json::json!({"sunrise_client::game::f": {
                "module": "sunrise_client::not a path!",
                "file": "crates/sunrise-client/src/game.rs", "span": [0, 1],
                "signature_hash": "", "sha256": "",
            }}),
        );
        let error = read().unwrap_err().to_string();
        assert!(error.contains("functions.json"), "{error}");
        assert!(
            error.contains("sunrise_client::game::f")
                && error.contains("sunrise_client::not a path!"),
            "{error}"
        );
    }

    #[test]
    fn patching_refuses_a_missing_loop_catalog_but_accepts_an_empty_one() {
        let game = tempfile::tempdir().unwrap();
        let commit = crate::tests::A;
        let error = super::loop_slots(game.path(), commit, &[], &Default::default())
            .err()
            .unwrap();
        assert!(error.starts_with("E_MIXIN_INDEX:"), "{error}");
        assert!(
            error.contains("loops.json") && error.contains("fetch the SDK again"),
            "{error}"
        );
        std::fs::write(
            game.path().join("loops.json"),
            format!(r#"{{"schema":1,"build":{{"commit":"{commit}"}},"loops":[]}}"#),
        )
        .unwrap();
        assert!(
            super::loop_slots(game.path(), commit, &[], &Default::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn generated_install_passes_owners_and_result_to_startup_gate() {
        assert_eq!(
            super::install_expression("game::tick", &["first", "second"], "patch::install()"),
            r#"("game::tick", &["first", "second"], patch::install())"#
        );
        // The patch library's entry module hands its installs to the loader, which passes them
        // to the startup gate before any mod's entries are registered.
        let template =
            include_str!("../../../crates/sunrise-client/src/native_mod_entry.rs.template");
        let installs = template.find("fn installs()").unwrap();
        assert!(template[installs..].find("/* MOD_REDIRECTS */").is_some());
    }

    #[test]
    fn target_refusal_names_every_requesting_row() {
        let refused = super::api_gen::patch::Refusal {
            code: "E_MIXIN_NOT_REPLACEABLE",
            target: "game::tick".into(),
            why: "no slot".into(),
        };
        let requests = vec![
            ("first".into(), "before".into(), "game::tick".into()),
            ("second".into(), "after".into(), "game::tick".into()),
            ("other".into(), "row".into(), "game::other".into()),
        ];
        assert_eq!(
            super::target_refusal(&refused, &requests),
            "E_MIXIN_NOT_REPLACEABLE: first:before, second:after: game::tick: no slot"
        );
    }
}
