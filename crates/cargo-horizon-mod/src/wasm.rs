//! `build --wasm`: the mod in the current folder built to `mod.wasm`, and `wasm-std`: the game's
//! standard library member.
//!
//! A wasm mod is a member of its dependency group, linked at load by the game
//! (`horizon-mod-wasm`): one memory, one copy of std and of each library mod. So every member is
//! a position-independent shared module built by one pinned toolchain, with std built from
//! source (`-Zbuild-std`) so its symbols are the same in every member. This tool is also the
//! link step (cargo calls it as the linker, with `HORIZON_MOD_LINK` set): the std member links
//! std whole and exports it; a mod leaves std and its library mods out, to resolve to theirs.
use horizon_mod_package::manifest::{MANIFEST, WASM, sha512_of};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The one toolchain every wasm mod and the game's std member are built with.
pub const TOOLCHAIN: &str = "1.99.1";
const TARGET: &str = "wasm32-unknown-unknown";
/// Set when cargo runs this tool as the linker: `std` or `member`.
pub const LINK_ENV: &str = "HORIZON_MOD_LINK";
const LLD_ENV: &str = "HORIZON_MOD_LLD";
/// The library mods' crate names, comma-separated, that a member leaves out.
const PROVIDED_ENV: &str = "HORIZON_MOD_PROVIDED";
const PROFILE: &str = "horizon-wasm";
/// The crates of std, linked whole into the std member and left out of every other member.
const STD_CRATES: &[&str] = &[
    "std",
    "alloc",
    "core",
    "panic_abort",
    "dlmalloc",
    "cfg_if",
    "rustc_demangle",
    "std_detect",
    "hashbrown",
    "rustc_std_workspace_alloc",
    "rustc_std_workspace_core",
    "miniz_oxide",
    "adler2",
    "unwind",
    "libc",
    "addr2line",
    "gimli",
    "object",
    "memchr",
    "compiler_builtins",
    "panic_unwind",
];

/// Whether a member exports `name` to its group: only the component's own exports (the mod
/// API's, `<interface>#<function>`, and the canonical ABI's `cabi_*`). Every other symbol a
/// crate exports (the guest glue's entry into the mod, a JS runtime's glue from a dependency
/// such as `getrandom`) is the member's own, and left out so the linker drops what only it
/// reached.
fn group_export(name: &str) -> bool {
    name.contains('#') || name.starts_with("cabi_")
}

/// The pinned toolchain's version line, its sysroot's linker, installing it first if needed.
struct Toolchain {
    version: String,
    lld: PathBuf,
}

/// `tool` (`cargo`, `rustc`) of the pinned toolchain: through rustup when there is one, else
/// the one on `PATH`, which must then be that toolchain.
fn pinned(tool: &str) -> Command {
    let mut command = if rustup_found() {
        let mut c = Command::new("rustup");
        c.args(["run", TOOLCHAIN, tool]);
        c
    } else {
        Command::new(tool)
    };
    command.env_remove("RUSTC").env_remove("RUSTC_WRAPPER");
    command
}

fn rustup_found() -> bool {
    Command::new("rustup").arg("--version").output().is_ok()
}

fn output(mut command: Command) -> Result<String, String> {
    let out = command.output().map_err(|e| format!("{command:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn toolchain() -> Result<Toolchain, String> {
    let version = || {
        output({
            let mut c = pinned("rustc");
            c.arg("-vV");
            c
        })
    };
    let info = match version() {
        Ok(info) => info,
        Err(_) if rustup_found() => {
            println!("Installing Rust {TOOLCHAIN} with rust-src for {TARGET}…");
            let status = Command::new("rustup")
                .args([
                    "toolchain",
                    "install",
                    TOOLCHAIN,
                    "--profile",
                    "minimal",
                    "--component",
                    "rust-src",
                    "--target",
                    TARGET,
                ])
                .status()
                .map_err(|e| format!("rustup: {e}"))?;
            if !status.success() {
                return Err(format!("rustup couldn't install Rust {TOOLCHAIN}"));
            }
            version()?
        }
        Err(e) => return Err(e),
    };
    let field = |name: &str| {
        info.lines()
            .find_map(|l| l.strip_prefix(name))
            .map(str::trim)
            .ok_or(format!("rustc -vV: no {name}"))
    };
    if field("release:")? != TOOLCHAIN {
        return Err(format!(
            "wasm mods build with Rust {TOOLCHAIN}, found {}: install rustup, or put {TOOLCHAIN} first on PATH",
            field("release:")?
        ));
    }
    let host = field("host:")?;
    let sysroot = output({
        let mut c = pinned("rustc");
        c.args(["--print", "sysroot"]);
        c
    })?;
    Ok(Toolchain {
        version: format!("{} {}", field("release:")?, field("commit-hash:")?),
        lld: Path::new(sysroot.trim()).join(format!("lib/rustlib/{host}/bin/rust-lld")),
    })
}

/// Builds the crate at `root` as a group member and gives its module.
fn cargo_member(
    root: &Path,
    target_dir: &Path,
    toolchain: &Toolchain,
    role: &str,
    provided: &[String],
    features: &[String],
) -> Result<Vec<u8>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = pinned("cargo");
    command
        .env_remove("HORIZON_MOD_INTERNALS_MANIFEST")
        .env_remove("HORIZON_MOD_INTERNALS_CATALOG");
    if role == "member" {
        let text = std::fs::read_to_string(root.join(MANIFEST)).map_err(|e| e.to_string())?;
        let manifest: toml_edit::DocumentMut =
            text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
        if manifest
            .get("native")
            .and_then(|n| n.get("internals"))
            .is_some_and(|i| {
                i.as_array().is_some_and(|a| !a.is_empty())
                    || i.as_array_of_tables().is_some_and(|a| !a.is_empty())
            })
        {
            command.env(
                "HORIZON_MOD_INTERNALS_MANIFEST",
                std::path::absolute(root.join(MANIFEST)).map_err(|e| e.to_string())?,
            );
            command.env(
                "HORIZON_MOD_INTERNALS_CATALOG",
                std::path::absolute(crate::internals::wasm_catalog(root)?)
                    .map_err(|e| e.to_string())?,
            );
        }
    }
    command.args(["rustc", "--lib", "--crate-type", "cdylib"]);
    if !features.is_empty() {
        command.args(["--features", &features.join(",")]);
    }
    command
        .args([
            "--target",
            TARGET,
            "--profile",
            PROFILE,
            "-Zbuild-std=std,panic_abort",
        ])
        .args([
            "--config",
            &format!("profile.{PROFILE}.inherits=\"release\""),
        ])
        .args(["--config", &format!("profile.{PROFILE}.panic=\"abort\"")])
        // One unit per crate: about 14% faster mod logic for a slower mod build. LTO isn't
        // possible, as each crate's symbols must stay visible to the group's linker.
        .args(["--config", &format!("profile.{PROFILE}.codegen-units=1")])
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("RUSTC_BOOTSTRAP", "1")
        // Every crate's symbols stay inside its member, but the member's own crate's, which its
        // dependents link against (`-- -Zdefault-visibility=interposable` below): a member exports
        // its own API, and the linker drops whatever only another crate's exports reached.
        .env(
            "RUSTFLAGS",
            "-Crelocation-model=pic -Zshare-generics=n -Zdefault-visibility=hidden",
        )
        .env("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER", exe)
        .env(LINK_ENV, role)
        .env(LLD_ENV, &toolchain.lld)
        .env(PROVIDED_ENV, provided.join(","));
    command.args(["--", "-Zdefault-visibility=interposable"]);
    let status = command.status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err("cargo build failed".into());
    }
    let metadata: Value = serde_json::from_str(&output({
        let mut c = pinned("cargo");
        c.args(["metadata", "--format-version", "1", "--no-deps"]);
        c.current_dir(root);
        c
    })?)
    .map_err(|e| e.to_string())?;
    let lib = lib_name(&metadata["packages"][0]).ok_or("the crate has no [lib]")?;
    let path = target_dir
        .join(TARGET)
        .join(PROFILE)
        .join(format!("{lib}.wasm"));
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn lib_name(package: &Value) -> Option<String> {
    package["targets"].as_array()?.iter().find_map(|t| {
        let lib = t["kind"]
            .as_array()?
            .iter()
            .any(|k| k == "lib" || k == "cdylib" || k == "rlib");
        lib.then(|| t["name"].as_str().map(|n| n.replace('-', "_")))?
    })
}

/// The game's std member, built into `target_dir`.
fn std_member(target_dir: &Path, toolchain: &Toolchain) -> Result<Vec<u8>, String> {
    let dir = target_dir.join("horizon-wasm-std");
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"horizon-wasm-std\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("src/lib.rs"),
        "// The group's one std: linked whole and exported.\n",
    )
    .map_err(|e| e.to_string())?;
    cargo_member(&dir, target_dir, toolchain, "std", &[], &[])
}

/// `wasm-std <out>`: the std member the game ships beside its executable.
pub fn wasm_std(args: &[String]) -> Result<(), String> {
    let out = args
        .first()
        .ok_or("cargo horizon-mod wasm-std <out.wasm>")?;
    let toolchain = toolchain()?;
    let target = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("target/horizon-mod");
    std::fs::write(out, std_member(&target, &toolchain)?).map_err(|e| format!("{out}: {e}"))
}

/// A library mod this mod depends on: its crate, and its built `mod.wasm`.
struct Library {
    id: String,
    /// Declared `optional = true`: the mod is built with the Cargo feature of its id.
    optional: bool,
    krate: String,
    module: Vec<u8>,
    sha512: String,
}

/// The mod's `[dependencies]` that are wasm library mods among its crate's dependencies.
fn libraries(root: &Path, manifest: &toml_edit::DocumentMut) -> Result<Vec<Library>, String> {
    let wanted: Vec<(&str, bool)> = manifest
        .get("dependencies")
        .and_then(|d| d.as_table_like())
        .map(|d| {
            d.iter()
                .map(|(id, item)| {
                    let optional = item
                        .as_table_like()
                        .and_then(|t| t.get("optional"))
                        .and_then(|o| o.as_bool());
                    (id, optional == Some(true))
                })
                .collect()
        })
        .unwrap_or_default();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let metadata: Value = serde_json::from_str(&output({
        let mut c = pinned("cargo");
        c.args(["metadata", "--format-version", "1"]);
        c.current_dir(root);
        c
    })?)
    .map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for package in metadata["packages"].as_array().into_iter().flatten() {
        let Some(dir) = package["manifest_path"]
            .as_str()
            .and_then(|p| Path::new(p).parent())
        else {
            continue;
        };
        // The mod itself has no `[wasm]` before its first build, so it isn't read as one.
        if dir.canonicalize().ok() == root.canonicalize().ok() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(dir.join(MANIFEST)) else {
            continue;
        };
        let theirs = horizon_mod_package::Manifest::parse(&text)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let Some(&(_, optional)) = wanted.iter().find(|(id, _)| *id == theirs.id) else {
            continue;
        };
        let wasm = theirs.wasm.ok_or_else(|| {
            format!(
                "{}: a wasm mod's dependency must be a wasm mod too",
                theirs.id
            )
        })?;
        let module = std::fs::read(dir.join(WASM))
            .map_err(|e| format!("{}: build it with --wasm first ({e})", theirs.id))?;
        wasm.check(&module)
            .map_err(|e| format!("{}: {e}; rebuild it with --wasm", theirs.id))?;
        found.push(Library {
            id: theirs.id,
            optional,
            krate: lib_name(package).ok_or("a library mod has no [lib]")?,
            module,
            sha512: wasm.sha512().to_owned(),
        });
    }
    Ok(found)
}

/// `build --wasm`: builds `mod.wasm` beside `horizon-mod.toml` and records it in `[wasm]`.
pub fn build(root: &Path) -> Result<(), String> {
    let manifest_path = root.join(MANIFEST);
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let mut manifest: toml_edit::DocumentMut =
        text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    // A wasm mod's `[native]` holds only declarations, never native code or build pins.
    if manifest
        .get("native")
        .and_then(toml_edit::Item::as_table_like)
        .is_some_and(|native| native.iter().any(|(key, _)| key != "internals"))
    {
        return Err(format!(
            "{MANIFEST}: a [native] mod builds through the launcher, not --wasm"
        ));
    }
    let id = manifest
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or(format!("{MANIFEST}: id missing"))?
        .to_owned();
    let toolchain = toolchain()?;
    let target = std::path::absolute(root.join("target/horizon-mod")).map_err(|e| e.to_string())?;
    let libraries = libraries(root, &manifest)?;
    let provided: Vec<String> = libraries.iter().map(|l| l.krate.clone()).collect();
    // An optional library is linked when installed and guarded at the call (`if_installed!`).
    let features: Vec<String> = libraries
        .iter()
        .filter(|l| l.optional)
        .map(|l| l.id.clone())
        .collect();
    let module = rename(
        &cargo_member(root, &target, &toolchain, "member", &provided, &features)?,
        &id,
    )?;
    // Linked here as the game will, so a member that can't link fails now, not on load.
    let std = std_member(&target, &toolchain)?;
    let mut linker = wit_component::Linker::default();
    linker.stub_missing_functions(true);
    linker
        .library("libstd.so", &std, false)
        .map_err(|e| format!("{e:#}"))?;
    for library in &libraries {
        linker
            .library(&format!("lib{}.so", library.id), &library.module, false)
            .map_err(|e| format!("{}: {e:#}", library.id))?;
    }
    linker
        .library(&format!("lib{id}.so"), &module, false)
        .map_err(|e| format!("{e:#}"))?;
    linker
        .encode()
        .map_err(|e| format!("{id} doesn't link with its group: {e:#}"))?;

    std::fs::write(root.join(WASM), &module).map_err(|e| format!("{WASM}: {e}"))?;
    let mut wasm = toml_edit::Table::new();
    wasm["sha512"] = toml_edit::value(sha512_of(&module));
    wasm["toolchain"] = toml_edit::value(&toolchain.version);
    if !libraries.is_empty() {
        let mut linked = toml_edit::Table::new();
        for library in &libraries {
            linked[&library.id] = toml_edit::value(&library.sha512);
        }
        wasm["linked"] = toml_edit::Item::Table(linked);
    }
    manifest["wasm"] = toml_edit::Item::Table(wasm);
    std::fs::write(&manifest_path, manifest.to_string()).map_err(|e| format!("{MANIFEST}: {e}"))
}

/// Gives the mod's exports of the mod API's interfaces the mod's own plain names,
/// `<id>-<interface>`, so the members of one group don't collide; the game finds them by
/// those names (`mod_api::wasm_host`).
fn rename(wasm: &[u8], id: &str) -> Result<Vec<u8>, String> {
    use wit_parser::{TypeOwner, WorldItem, WorldKey};
    let (_, mut bindgen) = wit_component::metadata::decode(wasm).map_err(|e| format!("{e:#}"))?;
    let resolve = &mut bindgen.resolve;
    let mut prefixes = Vec::new();
    let exports = std::mem::take(&mut resolve.worlds[bindgen.world].exports);
    let mut renamed = wit_parser::IndexMap::default();
    for (key, item) in exports {
        let (WorldKey::Interface(iface), WorldItem::Interface { .. }) = (&key, &item) else {
            renamed.insert(key, item);
            continue;
        };
        let full = resolve
            .id_of(*iface)
            .ok_or("an exported interface without a name")?;
        let short = resolve.interfaces[*iface]
            .name
            .clone()
            .ok_or("an unnamed interface")?;
        let name = format!(
            "{}-{short}",
            horizon_mod_package::manifest::export_prefix(id)
        );
        prefixes.push((format!("{full}#"), format!("{name}#")));
        // An unnamed copy of the interface, exported under the plain name: it owns its own
        // copies of the types it `use`s, and its functions point at them.
        let mut copy = resolve.interfaces[*iface].clone();
        copy.name = None;
        copy.types.clear();
        copy.functions.clear();
        let copy = resolve.interfaces.alloc(copy);
        let mut map = std::collections::HashMap::new();
        for (type_name, ty) in resolve.interfaces[*iface].types.clone() {
            let mut def = resolve.types[ty].clone();
            def.owner = TypeOwner::Interface(copy);
            let new = resolve.types.alloc(def);
            map.insert(ty, new);
            resolve.interfaces[copy].types.insert(type_name, new);
        }
        for (function_name, mut function) in resolve.interfaces[*iface].functions.clone() {
            for param in &mut function.params {
                param.ty = remap(resolve, &map, param.ty);
            }
            function.result = function.result.map(|ty| remap(resolve, &map, ty));
            resolve.interfaces[copy]
                .functions
                .insert(function_name, function);
        }
        renamed.insert(
            WorldKey::Name(name),
            WorldItem::Interface {
                id: copy,
                stability: Default::default(),
                span: Default::default(),
                docs: Default::default(),
                external_id: None,
            },
        );
    }
    // The world, its exports renamed, in a package of the mod's own: the linker merges its
    // members' packages by name, and theirs differ.
    let prefix = horizon_mod_package::manifest::export_prefix(id);
    let world = bindgen.world;
    resolve.worlds[world].exports = renamed;
    resolve.worlds[world].name = prefix.clone();
    let name = wit_parser::PackageName {
        namespace: "horizon-mod".into(),
        name: prefix.clone(),
        version: None,
    };
    let package = resolve.packages.alloc(wit_parser::Package {
        name: name.clone(),
        docs: Default::default(),
        interfaces: Default::default(),
        worlds: [(prefix, world)].into_iter().collect(),
    });
    resolve.package_names.insert(name, package);
    if let Some(old) = resolve.worlds[world].package {
        resolve.packages[old].worlds.retain(|_, w| *w != world);
    }
    resolve.worlds[world].package = Some(package);
    // The module again, its exports renamed and its old component-type sections dropped.
    let mut out = wasm_encoder::Module::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|e| e.to_string())?;
        match &payload {
            wasmparser::Payload::CustomSection(c) if c.name().starts_with("component-type") => {
                continue;
            }
            wasmparser::Payload::ExportSection(reader) => {
                let mut section = wasm_encoder::ExportSection::new();
                for export in reader.clone() {
                    let export = export.map_err(|e| e.to_string())?;
                    let mut name = export.name.to_owned();
                    'rename: for (from, to) in &prefixes {
                        for prefix in ["", "cabi_post_"] {
                            if let Some(rest) = name.strip_prefix(&format!("{prefix}{from}")) {
                                name = format!("{prefix}{to}{rest}");
                                break 'rename;
                            }
                        }
                    }
                    let kind = match export.kind {
                        wasmparser::ExternalKind::Func => wasm_encoder::ExportKind::Func,
                        wasmparser::ExternalKind::Table => wasm_encoder::ExportKind::Table,
                        wasmparser::ExternalKind::Memory => wasm_encoder::ExportKind::Memory,
                        wasmparser::ExternalKind::Global => wasm_encoder::ExportKind::Global,
                        _ => return Err(format!("unexpected export {name}")),
                    };
                    section.export(&name, kind, export.index);
                }
                out.section(&section);
                continue;
            }
            _ => {}
        }
        if let Some((id, range)) = payload.as_section() {
            out.section(&wasm_encoder::RawSection {
                id,
                data: &wasm[range.start as usize..range.end as usize],
            });
        }
    }
    let mut bytes = out.finish();
    wit_component::embed_component_metadata(
        &mut bytes,
        &bindgen.resolve,
        bindgen.world,
        wit_component::StringEncoding::UTF8,
        false,
    )
    .map_err(|e| format!("{e:#}"))?;
    Ok(bytes)
}

/// `ty` with the interface's own types replaced by the copy's, through the anonymous types
/// (`tuple<flow, …-out>`, `option<…>`) that hold them.
fn remap(
    resolve: &mut wit_parser::Resolve,
    map: &std::collections::HashMap<wit_parser::TypeId, wit_parser::TypeId>,
    ty: wit_parser::Type,
) -> wit_parser::Type {
    use wit_parser::{Type, TypeDefKind, TypeOwner};
    let Type::Id(id) = ty else {
        return ty;
    };
    if let Some(new) = map.get(&id) {
        return Type::Id(*new);
    }
    let def = resolve.types[id].clone();
    if def.owner != TypeOwner::None {
        return ty;
    }
    let mut each = |t: Type| remap(resolve, map, t);
    let kind = match def.kind.clone() {
        TypeDefKind::Tuple(mut tuple) => {
            tuple.types = tuple.types.into_iter().map(&mut each).collect();
            TypeDefKind::Tuple(tuple)
        }
        TypeDefKind::Option(t) => TypeDefKind::Option(each(t)),
        TypeDefKind::List(t) => TypeDefKind::List(each(t)),
        TypeDefKind::Map(k, v) => TypeDefKind::Map(each(k), each(v)),
        TypeDefKind::FixedLengthList(t, n) => TypeDefKind::FixedLengthList(each(t), n),
        TypeDefKind::Result(mut result) => {
            result.ok = result.ok.map(&mut each);
            result.err = result.err.map(&mut each);
            TypeDefKind::Result(result)
        }
        _ => return ty,
    };
    if kind == def.kind {
        return ty;
    }
    Type::Id(resolve.types.alloc(wit_parser::TypeDef { kind, ..def }))
}

/// The link step, run by cargo as the linker: rust-lld linking a shared module, then rustc's
/// own metadata sections dropped.
pub fn link(role: &str, args: Vec<String>) -> Result<(), String> {
    let lld = std::env::var(LLD_ENV).map_err(|_| format!("{LLD_ENV} unset"))?;
    let provided: Vec<String> = std::env::var(PROVIDED_ENV)
        .unwrap_or_default()
        .split(',')
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    let crate_of = |arg: &str| -> Option<String> {
        let file = Path::new(arg).file_name()?.to_str()?;
        let stem = file.strip_prefix("lib")?.strip_suffix(".rlib")?;
        Some(stem.rsplit_once('-')?.0.to_owned())
    };
    let mut args = args
        .into_iter()
        .flat_map(|arg| match arg.strip_prefix('@') {
            Some(file) => std::fs::read_to_string(file)
                .map(|t| t.lines().map(str::to_owned).collect())
                .unwrap_or_else(|_| vec![arg]),
            None => vec![arg],
        });
    let mut out = Vec::new();
    let mut output = None;
    while let Some(arg) = args.next() {
        let krate = crate_of(&arg);
        let std = krate.as_deref().is_some_and(|k| STD_CRATES.contains(&k));
        if arg == "-o" {
            output = args.next();
            out.extend(["-o".to_owned(), output.clone().unwrap_or_default()]);
        } else if role == "std" && std {
            out.extend([
                "--whole-archive".to_owned(),
                arg,
                "--no-whole-archive".to_owned(),
            ]);
        } else if role == "member"
            && krate.as_deref().is_some_and(|k| {
                (std && k != "compiler_builtins") || provided.iter().any(|p| p == k)
            })
        {
            continue;
        } else if role == "std" && arg == "--gc-sections" {
            out.push("--no-gc-sections".to_owned());
        } else if arg == "--export" {
            let name = args.next().unwrap_or_default();
            if role == "std" || group_export(&name) {
                out.extend([arg, name]);
            }
        } else {
            out.push(arg);
        }
    }
    out.extend(
        [
            "--shared",
            "--strip-all",
            "--unresolved-symbols=import-dynamic",
        ]
        .map(str::to_owned),
    );
    if role == "std" {
        out.push("--export-all".to_owned());
    }
    let status = Command::new(&lld)
        .args(&out)
        .status()
        .map_err(|e| format!("{lld}: {e}"))?;
    if !status.success() {
        return Err(format!("rust-lld failed ({status})"));
    }
    let output = output.ok_or("rustc gave no -o")?;
    let bytes = std::fs::read(&output).map_err(|e| e.to_string())?;
    let mut module = wasm_encoder::Module::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        let payload = payload.map_err(|e| e.to_string())?;
        if let wasmparser::Payload::CustomSection(c) = &payload
            && c.name().starts_with(".rmeta")
        {
            continue;
        }
        // A member imports the mod API (`<package>:<name>/…`) and its group (`env`, `GOT.*`).
        // Any other import a crate declared (a JS runtime's, from `getrandom` under `zip`,
        // some under `env` itself) is never called in the game: it becomes a group import the
        // linker stubs to trap, named for this member, as the linker makes one stub per
        // importer and two members' would collide.
        if let wasmparser::Payload::ImportSection(reader) = &payload {
            let member = Path::new(&output)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            let mut section = wasm_encoder::ImportSection::new();
            for import in reader.clone().into_imports() {
                let import = import.map_err(|e| e.to_string())?;
                let js = import.name.starts_with("__wbindgen") || import.name.starts_with("__wbg");
                let ours = !js
                    && (import.module == "env"
                        || import.module.starts_with("GOT.")
                        || import.module.contains(':'));
                let ty =
                    wasm_encoder::EntityType::try_from(import.ty).map_err(|e| format!("{e:?}"))?;
                if ours {
                    section.import(import.module, import.name, ty);
                } else {
                    let name = format!("{member}:{}.{}", import.module, import.name);
                    section.import("env", &name, ty);
                }
            }
            module.section(&section);
            continue;
        }
        // `--shared` exports every visible symbol; a member keeps only its group exports and
        // the linker's own (`__wasm_*`), so two members' copies of a crate's `#[no_mangle]`
        // items (a JS runtime's glue, the guest entry) don't collide in one group.
        if let wasmparser::Payload::ExportSection(reader) = &payload
            && role != "std"
        {
            let mut section = wasm_encoder::ExportSection::new();
            for export in reader.clone() {
                let export = export.map_err(|e| e.to_string())?;
                if group_export(export.name) || export.name.starts_with("__wasm_") {
                    section.export(export.name, export.kind.into(), export.index);
                }
            }
            module.section(&section);
            continue;
        }
        if let Some((id, range)) = payload.as_section() {
            module.section(&wasm_encoder::RawSection {
                id,
                data: &bytes[range.start as usize..range.end as usize],
            });
        }
    }
    std::fs::write(&output, module.finish()).map_err(|e| e.to_string())
}
