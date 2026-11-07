//! Tool-written internal declarations and the old/new pin's signature diff.
use horizon_mod_package::{
    Manifest,
    index::native,
    internals::{self, ListedInternal},
    manifest::MANIFEST,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Value};

const USAGE: &str =
    "usage: cargo horizon-mod internals sync [--repo <path>] | add <path>… [--kind <kind>]";
const CATALOG: &str = "internals.json";
const CATALOG_LIMIT: u64 = 64 << 20;
const ARCHIVE_LIMIT: u64 = 4 << 30;

pub fn run(args: &[String]) -> Result<(), String> {
    let Some((command, args)) = args.split_first() else {
        return Err(USAGE.into());
    };
    if command == "sync" {
        let repo = match args {
            [] => None,
            [flag, path] if flag == "--repo" && !path.starts_with('-') => Some(Path::new(path)),
            _ => return Err(USAGE.into()),
        };
        return sync(Path::new("."), repo);
    }
    let mut paths = Vec::new();
    let mut kind = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--kind" && kind.is_none() {
            kind = Some(
                args.next()
                    .filter(|k| !k.starts_with('-'))
                    .ok_or(USAGE)?
                    .as_str(),
            );
        } else if arg.starts_with('-') {
            return Err(USAGE.into());
        } else {
            paths.push(arg.as_str());
        }
    }
    if command != "add" || paths.is_empty() {
        return Err(USAGE.into());
    }
    add(Path::new("."), &paths, kind)
}

fn add(dir: &Path, paths: &[&str], kind: Option<&str>) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest = Manifest::parse(&text)?;
    let (declared, catalog) = if let Some(wasm) = &manifest.wasm {
        (wasm.internals(), internals::read(&wasm_catalog(dir)?)?)
    } else if let Some(native) = &manifest.native {
        (native.internals.as_slice(), load(dir, &native.horizon)?)
    } else {
        return Err("internals add needs a [native] or [wasm] mod".into());
    };
    // Resolve every request before editing: any unavailable or ambiguous path leaves the file alone.
    let entries = paths
        .iter()
        .map(|path| {
            internals::resolve(&catalog, &manifest.id, path, kind).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    upsert(&mut doc, declared, &entries)?;
    for entry in entries {
        println!("Added internal {}", entry.path);
    }
    fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Adds or refreshes `entries` in `[native].internals`, which also holds a `[wasm]` mod's.
fn upsert(
    doc: &mut DocumentMut,
    declared: &[horizon_mod_package::manifest::Internal],
    entries: &[&ListedInternal],
) -> Result<(), String> {
    let mut listed: BTreeMap<_, _> = declared
        .iter()
        .enumerate()
        .map(|(index, entry)| ((entry.path.as_str(), entry.kind.as_deref()), index))
        .collect();
    for entry in entries {
        let key = (entry.path.as_str(), Some(entry.kind.as_str()));
        let existing = listed
            .get(&key)
            .or_else(|| listed.get(&(entry.path.as_str(), None)))
            .copied();
        if let Some(index) = existing {
            set_fingerprint(doc, index, &entry.hash, Some(&entry.kind))?;
            if listed.get(&(entry.path.as_str(), None)) == Some(&index) {
                listed.remove(&(entry.path.as_str(), None));
            }
            listed.insert(key, index);
        } else {
            let item = &mut doc["native"]["internals"];
            if item.is_none() {
                *item = toml_edit::value(toml_edit::Array::new());
            }
            // The manifest accepts both inline arrays and [[native.internals]] tables.
            if let Some(array) = item.as_array_mut() {
                let mut internal = toml_edit::InlineTable::new();
                internal.insert("path", entry.path.as_str().into());
                internal.insert("hash", entry.hash.as_str().into());
                internal.insert("kind", entry.kind.as_str().into());
                internal.fmt();
                array.push(internal);
            } else if let Some(array) = item.as_array_of_tables_mut() {
                let mut internal = toml_edit::Table::new();
                internal["path"] = toml_edit::value(&entry.path);
                internal["hash"] = toml_edit::value(&entry.hash);
                internal["kind"] = toml_edit::value(&entry.kind);
                array.push(internal);
            } else {
                return Err(format!("{MANIFEST}: native.internals must be an array"));
            }
            listed.insert(key, listed.len());
        }
    }
    Ok(())
}

/// The launcher owns source staging and compiler diagnostics; this tool owns manifest edits.
fn sync(dir: &Path, repo: Option<&Path>) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let original = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest = Manifest::parse(&original)?;
    if let Some(wasm) = &manifest.wasm {
        let repo = match repo {
            Some(repo) => repo.to_path_buf(),
            None => dir
                .canonicalize()
                .map_err(|e| e.to_string())?
                .ancestors()
                .find(|path| {
                    path.join("Cargo.toml").is_file()
                        && path.join("crates/horizon-mod-api-gen").is_dir()
                })
                .ok_or("Could not find a Horizon workspace; pass --repo <path>")?
                .to_path_buf(),
        };
        let directory = repo.join("crates/horizon-mod-api-gen/wasm-internals");
        if !repo.join("Cargo.toml").is_file() || !directory.is_dir() {
            return Err(format!("{}: not a Horizon workspace", repo.display()));
        }
        let output = directory.join(format!("{}.txt", manifest.id));
        let paths: BTreeSet<&str> = wasm
            .internals()
            .iter()
            .map(|entry| entry.path.as_str())
            .chain(
                manifest
                    .inject
                    .iter()
                    .filter(|i| {
                        i.access == horizon_mod_package::manifest::inject::Access::Internals
                    })
                    .map(|entry| entry.at.as_str()),
            )
            .collect();
        let mut contents = format!(
            "# {}: written by cargo horizon-mod internals sync from horizon-mod.toml\n",
            manifest.id
        );
        for path in paths {
            contents.push_str(path);
            contents.push('\n');
        }
        let changed = match fs::read(&output) {
            Ok(old) => old != contents.as_bytes(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(format!("{}: {error}", output.display())),
        };
        if changed {
            atomic_write(&output, contents)?;
        }
        println!(
            "{}: {}",
            output.display(),
            if changed { "changed" } else { "unchanged" }
        );
        return Ok(());
    }
    if repo.is_some() {
        return Err(USAGE.into());
    }
    manifest
        .native
        .as_ref()
        .ok_or("internals sync needs a [native] or [wasm] mod")?;
    let launcher =
        std::env::var_os("HORIZON_LAUNCHER").unwrap_or_else(|| "horizon-launcher".into());
    let output = std::process::Command::new(launcher)
        .args(["native-build", "--sync-internals", "--trust-native-code"])
        .arg(dir.canonicalize().map_err(|e| e.to_string())?)
        .stderr(std::process::Stdio::inherit())
        .output().map_err(|e| format!("Could not run horizon-launcher: {e}. Install it on PATH or set HORIZON_LAUNCHER to its executable."))?;
    if !output.status.success() {
        return Err("Internals discovery failed; the manifest was not changed.".into());
    }
    let entries: Vec<ListedInternal> = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid internals discovery result: {e}"))?;
    let (candidate, report) = replace(&original, &entries)?;
    // The explicit check may take time. Do not overwrite edits made while it was running.
    if fs::read_to_string(&path).map_err(|e| e.to_string())? != original {
        return Err("horizon-mod.toml changed during discovery; run cargo horizon-mod internals sync again.".into());
    }
    atomic_write(&path, candidate)?;
    print!("{report}");
    Ok(())
}

fn atomic_write(path: &Path, contents: String) -> Result<(), String> {
    let temporary = path.with_extension("toml.sync");
    fs::write(&temporary, contents)
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("{}: {e}", path.display())
        })
}

fn replace(text: &str, entries: &[ListedInternal]) -> Result<(String, String), String> {
    let manifest = Manifest::parse(text)?;
    let native = manifest
        .native
        .as_ref()
        .ok_or("internals sync needs a [native] mod")?;
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    // Each used access keeps at most one old row: an exact typed row first, then an untyped
    // row for its path. Every other row is dropped, so no stale duplicate survives the sync.
    let mut unclaimed: BTreeSet<_> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.kind.as_str()))
        .collect();
    let mut keep = vec![false; native.internals.len()];
    for typed in [true, false] {
        for (index, old) in native.internals.iter().enumerate() {
            if old.kind.is_some() != typed {
                continue;
            }
            let claim = unclaimed
                .iter()
                .find(|(path, kind)| {
                    *path == old.path && old.kind.as_deref().is_none_or(|k| k == *kind)
                })
                .copied();
            if let Some(claim) = claim {
                unclaimed.remove(&claim);
                keep[index] = true;
            }
        }
    }
    let mut report = String::new();
    for (index, old) in native.internals.iter().enumerate().rev() {
        if !keep[index] {
            let item = &mut doc["native"]["internals"];
            if let Some(array) = item.as_array_mut() {
                // TOML stores a trailing comment after the previous comma as this
                // value's prefix. Keep it when dropping the declaration that follows it.
                let removed = array.remove(index);
                let prefix = removed
                    .decor()
                    .prefix()
                    .and_then(|raw| raw.as_str())
                    .unwrap_or("");
                if let Some(next) = array.get_mut(index) {
                    let next_prefix = next
                        .decor()
                        .prefix()
                        .and_then(|raw| raw.as_str())
                        .unwrap_or("");
                    let combined = format!("{prefix}{next_prefix}");
                    next.decor_mut().set_prefix(combined);
                } else {
                    let trailing = array.trailing().as_str().unwrap_or("");
                    array.set_trailing(format!("{prefix}{trailing}"));
                }
            } else if let Some(array) = item.as_array_of_tables_mut() {
                array.remove(index);
            }
            report.push_str(&format!("removed: {}\n", old.path));
        }
    }
    let retained = Manifest::parse(&doc.to_string())?;
    for (path, kind) in &unclaimed {
        report.push_str(&format!("added: {path} ({kind})\n"));
    }
    let entries: Vec<_> = entries.iter().collect();
    upsert(
        &mut doc,
        &retained.native.as_ref().expect("native manifest").internals,
        &entries,
    )?;
    Ok((doc.to_string(), report))
}

/// Report all declarations, leaving unavailable ones declared so a build still refuses them.
/// Only resolvable entries receive the new game's fingerprint.
pub fn refresh(
    manifest: &Manifest,
    doc: &mut DocumentMut,
    old: &[ListedInternal],
    new: &[ListedInternal],
) -> Result<String, String> {
    let native = manifest
        .native
        .as_ref()
        .ok_or("update needs a [native] mod")?;
    let mut report = String::new();
    for (index, declared) in native.internals.iter().enumerate() {
        let path = &declared.path;
        let entry = match internals::resolve(new, &manifest.id, path, declared.kind.as_deref()) {
            Ok(entry) => entry,
            Err(error) => {
                report.push_str(&format!("missing: {path} ({error})\n"));
                continue;
            }
        };
        match internals::resolve(old, &manifest.id, path, declared.kind.as_deref()) {
            Ok(previous) if previous.hash == entry.hash => {
                report.push_str(&format!("unchanged: {path}\n"));
            }
            previous => {
                let signature =
                    previous.map_or("<unavailable in old game>", |e| e.signature.as_str());
                report.push_str(&format!(
                    "changed: {path}\n  {signature} → {}\n",
                    entry.signature
                ));
            }
        }
        set_fingerprint(doc, index, &entry.hash, declared.kind.as_deref())?;
    }
    Ok(report)
}

fn set_fingerprint(
    doc: &mut DocumentMut,
    index: usize,
    hash: &str,
    kind: Option<&str>,
) -> Result<(), String> {
    let item = &mut doc["native"]["internals"];
    let value = if let Some(array) = item.as_array_mut() {
        array
            .get_mut(index)
            .and_then(Value::as_inline_table_mut)
            .and_then(|i| {
                if let Some(kind) = kind {
                    set_string(i.entry("kind").or_insert(kind.into()), kind);
                }
                i.get_mut("hash")
            })
    } else {
        item.as_array_of_tables_mut()
            .and_then(|array| array.get_mut(index))
            .and_then(|i| {
                if let Some(kind) = kind {
                    let value = i.entry("kind").or_insert(toml_edit::value(kind));
                    set_string(
                        value.as_value_mut().expect("manifest kind is a string"),
                        kind,
                    );
                }
                i.get_mut("hash")
            })
            .and_then(Item::as_value_mut)
    }
    .ok_or_else(|| format!("{MANIFEST}: internal {index} has no hash"))?;
    set_string(value, hash);
    Ok(())
}

/// Replacing only the value keeps whitespace and comments around tool-written strings.
pub fn set_string(value: &mut Value, text: &str) {
    if value.as_str() == Some(text) {
        return;
    }
    let decor = value.decor().clone();
    *value = Value::from(text);
    *value.decor_mut() = decor;
}

/// The catalog a pinless wasm build uses, shared by `internals add` and the guest view build.
/// An explicit catalog is useful for a build against a staged game instead of a daily release.
pub(crate) fn wasm_catalog(dir: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("HORIZON_MOD_INTERNALS_CATALOG") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("HORIZON_MOD_INTERNALS_CATALOG must be an absolute path".into());
        }
        return Ok(path);
    }
    let revision = crate::pin(None)?;
    load(dir, &revision)?;
    Ok(dir
        .join("target/horizon-mod/prebuilt")
        .join(revision)
        .join(CATALOG))
}

/// The same release tags and archive layout the launcher reads. Keep each pin's catalog in
/// the mod's build folder, so updating can still show the old signature after a release expires.
pub fn load(dir: &Path, revision: &str) -> Result<Vec<ListedInternal>, String> {
    let cache = dir.join("target/horizon-mod/prebuilt").join(revision);
    let path = cache.join(CATALOG);
    if path.is_file() {
        return internals::read(&path);
    }
    let host = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return Err("no prebuilt game for this computer".into()),
    };
    let name = native::prebuilt_archive(host, false);
    let release = format!(
        "https://github.com/{}/releases/download/{}{revision}",
        native::PREBUILT_REPO,
        native::PREBUILT_TAG
    );
    let get = |file: &str| {
        let url = format!("{release}/{file}");
        ureq::get(&url).call().map_err(|e| format!("{url}: {e}"))
    };
    let sums = get("SHA256SUMS")?
        .into_string()
        .map_err(|e| e.to_string())?;
    let expected = sums
        .lines()
        .find_map(|line| {
            let (hash, file) = line.split_once(char::is_whitespace)?;
            (file.trim().trim_start_matches('*') == name).then_some(hash)
        })
        .ok_or_else(|| format!("{release}/SHA256SUMS: no checksum for {name}"))?;
    println!("Reading internals from the prebuilt game at {revision}");
    cache_catalog(&cache, &name, get(&name)?.into_reader(), expected)
}

/// Verify the whole archive before trusting any catalog bytes; never unpack its other files.
fn cache_catalog(
    cache: &Path,
    name: &str,
    body: impl Read,
    expected: &str,
) -> Result<Vec<ListedInternal>, String> {
    fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let path = cache.join(CATALOG);
    let archive = cache.join(format!("{name}.{}.partial", std::process::id()));
    let result = (|| {
        let mut body = body.take(ARCHIVE_LIMIT + 1);
        let mut file = fs::File::create(&archive).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut size = 0;
        let mut buffer = [0; 65536];
        loop {
            let read = body.read(&mut buffer).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            size += read as u64;
            if size > ARCHIVE_LIMIT {
                return Err(format!("{name}: prebuilt archive exceeds 4 GiB"));
            }
            hash.update(&buffer[..read]);
            file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
        }
        drop(file);
        if format!("{:x}", hash.finalize()) != expected {
            return Err(format!(
                "The prebuilt game {name} doesn't match its release's checksum. Try again."
            ));
        }
        let file = fs::File::open(&archive).map_err(|e| e.to_string())?;
        let text = catalog_from_archive(file)?;
        let entries = serde_json::from_str(&text).map_err(|e| format!("{CATALOG}: {e}"))?;
        let partial = path.with_extension(format!("json.{}.partial", std::process::id()));
        fs::write(&partial, text).map_err(|e| format!("{}: {e}", partial.display()))?;
        fs::rename(&partial, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(entries)
    })();
    let _ = fs::remove_file(&archive);
    result
}

fn catalog_from_archive(reader: impl Read) -> Result<String, String> {
    let reader = flate2::read::GzDecoder::new(reader).take(8 << 30);
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?;
        if path != Path::new(CATALOG) && path != Path::new("./internals.json") {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(format!("{CATALOG}: not a file in the prebuilt archive"));
        }
        let mut text = String::new();
        entry
            .take(CATALOG_LIMIT + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() as u64 > CATALOG_LIMIT {
            return Err(format!("{CATALOG}: catalog exceeds 64 MiB"));
        }
        return Ok(text);
    }
    Err(format!("prebuilt archive has no {CATALOG}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(path: &str, kind: tar::EntryType, contents: &str) -> Vec<u8> {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(kind);
        header.set_cksum();
        archive
            .append_data(&mut header, path, contents.as_bytes())
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn sync_replaces_declarations_and_preserves_retained_formatting_for_both_array_forms() {
        let header = format!(
            "# Mod\nid='example'\nname='Example'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native] # settings\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let entries: Vec<ListedInternal> = serde_json::from_str(r#"[{"path":"Used","kind":"struct","signature":"struct Used","hash":"new"},{"path":"New","kind":"fn","signature":"fn new()","hash":"added"}]"#).unwrap();
        for body in [
            "internals = [\n  { path = 'Used', hash = 'old' }, # keep\n  { path = 'Unused', hash = 'gone' },\n]\n",
            "[[native.internals]] # keep\npath = 'Used'\nhash = 'old' # hash\n[[native.internals]]\npath = 'Unused'\nhash = 'gone'\n",
        ] {
            let (candidate, report) = replace(&(header.clone() + body), &entries).unwrap();
            let native = Manifest::parse(&candidate).unwrap().native.unwrap();
            assert_eq!(native.internals.len(), 2);
            assert_eq!(native.internals[0].hash, "new");
            assert_eq!(native.internals[0].kind.as_deref(), Some("struct"));
            assert!(candidate.contains("# keep"));
            assert!(candidate.starts_with(&header));
            assert!(report.contains("removed: Unused"));
            assert!(report.contains("added: New (fn)"));
            assert!(!report.contains("added: Used"));
            let (unchanged, report) = replace(&candidate, &entries).unwrap();
            assert_eq!(unchanged, candidate);
            assert!(report.is_empty());
            let (empty, report) = replace(&candidate, &[]).unwrap();
            assert!(
                Manifest::parse(&empty)
                    .unwrap()
                    .native
                    .unwrap()
                    .internals
                    .is_empty()
            );
            assert!(report.contains("removed: Used"));
        }
        // An untyped and a typed row for one access leave one current declaration.
        let body = "internals = [{ path = 'Used', hash = 'stale' }, { path = 'Used', kind = 'struct', hash = 'stale' }]\n";
        let (candidate, _) = replace(&(header.clone() + body), &entries[..1]).unwrap();
        let native = Manifest::parse(&candidate).unwrap().native.unwrap();
        assert_eq!(native.internals.len(), 1);
        assert_eq!(native.internals[0].hash, "new");
        assert_eq!(native.internals[0].kind.as_deref(), Some("struct"));
    }

    #[test]
    fn reads_only_the_root_catalog_from_the_prebuilt_archive() {
        let catalog = include_str!("../tests/fixtures/internals-old.json");
        let bytes = archive("./internals.json", tar::EntryType::Regular, catalog);
        assert_eq!(catalog_from_archive(bytes.as_slice()).unwrap(), catalog);
        let bytes = archive("deps/internals.json", tar::EntryType::Regular, catalog);
        assert_eq!(
            catalog_from_archive(bytes.as_slice()).unwrap_err(),
            "prebuilt archive has no internals.json"
        );
        let bytes = archive("internals.json", tar::EntryType::Symlink, "");
        assert!(
            catalog_from_archive(bytes.as_slice())
                .unwrap_err()
                .contains("not a file")
        );
    }

    #[test]
    fn downloaded_catalog_is_cached_only_after_checksum_and_json_checks() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/i3-d/tests")
            .join(format!("archive-cache-{}", std::process::id()));
        let catalog = include_str!("../tests/fixtures/internals-old.json");
        let bytes = archive("./internals.json", tar::EntryType::Regular, catalog);
        let error = cache_catalog(&dir, "game.tar.gz", bytes.as_slice(), "wrong").unwrap_err();
        assert!(error.contains("doesn't match its release's checksum"));
        assert!(!dir.join(CATALOG).exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);

        let invalid = archive("./internals.json", tar::EntryType::Regular, "not JSON");
        let hash = format!("{:x}", Sha256::digest(&invalid));
        assert!(cache_catalog(&dir, "game.tar.gz", invalid.as_slice(), &hash).is_err());
        assert!(!dir.join(CATALOG).exists());

        let hash = format!("{:x}", Sha256::digest(&bytes));
        let entries = cache_catalog(&dir, "game.tar.gz", bytes.as_slice(), &hash).unwrap();
        assert_eq!(entries[0].path, "horizon_client::game::Game");
        assert_eq!(fs::read_to_string(dir.join(CATALOG)).unwrap(), catalog);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tools_and_launcher_share_the_freshness_check() {
        let entries: Vec<ListedInternal> =
            serde_json::from_str(include_str!("../tests/fixtures/internals-new.json")).unwrap();
        let mut declared = horizon_mod_package::manifest::Internal {
            path: "horizon_client::game::Game<'_>::reach".into(),
            kind: None,
            hash: "2222".into(),
        };
        assert_eq!(
            internals::check(&entries, "example", &declared)
                .unwrap_err()
                .to_string(),
            "E_INTERNAL_CHANGED: example uses horizon_client::game::Game<'_>::reach, whose signature is now impl Game<'_> { fn reach(&self, hand: Hand) -> f64 }; run cargo horizon-mod update to see what changed."
        );
        declared.hash = "abcd".into();
        internals::check(&entries, "example", &declared).unwrap();
    }
}
