//! `sunrise-stage ROOT INTERNALS_JSON COMMIT`: expose internals and give functions their replacement slots in a staged
//! source copy, then write the build data (`internals.json`, `loops.json`, `functions.json`) with the
//! header naming COMMIT, the source's full commit sha (#589). Never run on a working tree.
//!
//! `sunrise-stage surface ROOT OUT [--with-loops] [--with-internals]`:
//! write the SDK surface (`surface.json`) of the source tree at ROOT to OUT, and nothing else. It
//! only reads the tree, so a working tree is fine. The engine options are for comparisons: choose
//! the same options for both exports. `--with-internals` also includes loops.
//!
//! `sunrise-stage surface-catalogs ROOT SURFACE [--with-internals]`: add the source tree's
//! engine catalogs to an existing SDK export, without regenerating its typed API records.
//!
//! `sunrise-stage function-index ROOT OUT`: write the pristine source index, with published loop copies.
//!
//! `sunrise-stage uses SURFACE MOD_DIR [--json OUT --made-for BUILD]`: list
//! what the mod in MOD_DIR uses of SURFACE, each item with the place that brought it in, then
//! how it builds each struct it builds with a literal; with `--json`, write the table a package
//! carries.
//!
//! `sunrise-stage fits USES_JSON TARGET_SURFACE`: whether a uses table fits a build's surface,
//! naming each changed item. Exit 0 fits, 1 does not fit, 2 error.
//!
//! `sunrise-stage surface-diff OLD NEW [--notes DIR] [--base-notes DIR] [--api-notes-only] [--landed SHA] [--covered-by DIR --covered-base DIR] [--json OUT] [--gate]`:
//! print what changed between two surface exports, breaking changes first. `--notes` attaches
//! the change notes of DIR (`crates/sunrise-mod-vocab/changes`) whose file names are not in
//! `--base-notes` (the base tree's folder; required with `--notes`). For an SDK comparison,
//! `--api-notes-only` excludes engine ids from note attachment and stale-id checking. `--covered-by` attaches the
//! notes of its DIR not in `--covered-base` (a landed merge's own notes, against its first parent's) as coverage
//! only: their ids no longer read as unnoted, and an id among them that did not change is not stale. Exits 1 when a new note
//! names an id that did not change, or with `--gate` when a breaking change has no note; 2 when
//! an input cannot be read or the options are wrong.
use mod_source::mod_uses;
use mod_surface::Surface;
use mod_surface::uses::{Fit, ManifestFacts, Origin, UsesTable, uses_contained};
use std::path::{Path, PathBuf};
use sunrise_mod_api_gen::staging::{self, Mode};
use sunrise_mod_api_gen::surface::{
    ExportOptions, engine_catalogs_with_layout, export_with_layout,
};
use sunrise_mod_api_gen::{SourceLayout, functions};
use sunrise_mod_package::manifest::{MANIFEST, Manifest};
use sunrise_mod_package::native_route;

fn surface(args: &[PathBuf]) {
    let [root, out, flags @ ..] = args else {
        fail("usage: sunrise-stage surface ROOT OUT [--with-loops] [--with-internals]")
    };
    let mut options = ExportOptions::default();
    for flag in flags {
        match flag.to_str() {
            Some("--with-loops") => options.with_loops = true,
            Some("--with-internals") => options.with_internals = true,
            _ => fail(format!("unknown option {}", flag.display())),
        }
    }
    let started = std::time::Instant::now();
    // The generator writes source paths into `#[path]`s it then reads: they must be absolute.
    let root = match std::fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("sunrise-stage surface: {}: {error}", root.display());
            std::process::exit(1);
        }
    };
    let export = match SourceLayout::read(&root)
        .and_then(|layout| export_with_layout(&root, layout, options))
    {
        Ok(export) => export,
        Err(error) => {
            eprintln!("sunrise-stage surface: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = std::fs::write(out, export.surface.to_json()) {
        eprintln!("sunrise-stage surface: {}: {error}", out.display());
        std::process::exit(1);
    }
    for note in &export.notes {
        eprintln!("unclassified: {note}");
    }
    let counts: Vec<String> = export
        .counts()
        .iter()
        .map(|(kind, count)| format!("{kind}={count}"))
        .collect();
    println!(
        "surface {} unclassified={} hash={}",
        counts.join(" "),
        export.notes.len(),
        export.surface.hash
    );
    println!(
        "stage_phase=surface seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
}

/// Prints `message` as an error and exits 2.
fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("sunrise-stage: {message}");
    std::process::exit(2)
}

fn read_surface(path: &std::path::Path) -> Surface {
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    Surface::read(&bytes).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
}

/// `surface-read IN OUT`: IN as this tool reads it, written to OUT (the fields and collections it
/// doesn't know left out). Refuses a document it can't read, or whose stored hashes are not the
/// ones it gives their contracts ([`Surface::read_sealed`]).
fn surface_read(args: &[PathBuf]) -> ! {
    let [input, out] = args else {
        fail("usage: sunrise-stage surface-read IN OUT")
    };
    let bytes = std::fs::read(input).unwrap_or_else(|e| fail(format!("{}: {e}", input.display())));
    let (surface, unsealed) =
        Surface::read_sealed(&bytes).unwrap_or_else(|e| fail(format!("{}: {e}", input.display())));
    if !unsealed.is_empty() {
        fail(format!(
            "{}: these hashes are not what their contracts give: {}",
            input.display(),
            unsealed.join(", ")
        ));
    }
    std::fs::write(out, surface.to_json())
        .unwrap_or_else(|e| fail(format!("{}: {e}", out.display())));
    std::process::exit(0)
}

fn surface_catalogs(args: &[PathBuf]) {
    let [root, path, flags @ ..] = args else {
        fail("usage: sunrise-stage surface-catalogs ROOT SURFACE [--with-internals]");
    };
    let with_internals = match flags {
        [] => false,
        [flag] if flag == "--with-internals" => true,
        _ => fail("usage: sunrise-stage surface-catalogs ROOT SURFACE [--with-internals]"),
    };
    let surface = read_surface(path);
    let layout = SourceLayout::read(root).unwrap_or_else(|error| fail(error));
    let items = engine_catalogs_with_layout(root, layout, with_internals)
        .unwrap_or_else(|error| fail(error));
    std::fs::write(path, surface.with_engine_items(items).to_json())
        .unwrap_or_else(|error| fail(format!("{}: {error}", path.display())));
}

fn function_index(args: &[PathBuf]) {
    let [root, out] = args else {
        fail("usage: sunrise-stage function-index ROOT OUT");
    };
    let root = std::fs::canonicalize(root).unwrap_or_else(|error| fail(error));
    let layout = SourceLayout::read(&root).unwrap_or_else(|error| fail(error));
    let index = functions::index_with_layout(&root, layout).unwrap_or_else(|error| fail(error));
    std::fs::write(
        out,
        serde_json::to_vec(&index).expect("function index serializes"),
    )
    .unwrap_or_else(|error| fail(format!("{}: {error}", out.display())));
}

fn read_manifest_facts(dir: &Path) -> Result<ManifestFacts, String> {
    let path = dir.join(MANIFEST);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(format!("{MANIFEST}: not a regular file")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ManifestFacts::default());
        }
        Err(e) => return Err(format!("{MANIFEST}: {e}")),
    }
    let file = std::fs::File::open(path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest = Manifest::read(file)?;
    if native_route::manifest_game_code(&manifest).is_some() {
        return Err(
            "the mod declares game code (a [native] row or internals), which a surface alone cannot judge: use `cargo sunrise-mod check --build <name>`".into(),
        );
    }
    Ok((&manifest).into())
}

#[cfg(test)]
#[test]
fn only_an_absent_manifest_allows_a_source_only_scan() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.join(MANIFEST);
    assert_eq!(read_manifest_facts(root).unwrap(), ManifestFacts::default());

    std::fs::create_dir(&path).unwrap();
    assert!(read_manifest_facts(root).unwrap_err().starts_with(MANIFEST));

    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, "").unwrap();
    assert!(
        read_manifest_facts(&path)
            .unwrap_err()
            .starts_with(MANIFEST)
    );
}

#[cfg(test)]
#[test]
fn manifest_over_64_kib_uses_the_same_default_capability_as_a_native_package() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let manifest = format!(
        "id = 'large_meta'\nname = 'Large Meta'\nversion = '0.1.0'\ndescription = '{}'\n\
         [native]\ngame = '26.3'\nsunrise = '{}'\n",
        "m".repeat(64 << 10),
        "a".repeat(40)
    );
    std::fs::write(root.join(MANIFEST), manifest).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = 'large_meta'\n").unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();

    let package = sunrise_mod_package::Package::read(root).unwrap();
    let standalone = read_manifest_facts(root).unwrap();
    assert_eq!(standalone.names, ["registries"]);
    assert_eq!(standalone, ManifestFacts::from(&package.manifest));
}

fn uses_command(entry: mod_surface::child::ChildEntry, args: &[PathBuf]) {
    let [surface, dir, options @ ..] = args else {
        fail("usage: sunrise-stage uses SURFACE MOD_DIR [--json OUT --made-for BUILD]")
    };
    let (mut json, mut made_for) = (None, None);
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let value = options
            .next()
            .unwrap_or_else(|| fail(format!("{} needs a value", option.display())));
        match option.to_str() {
            Some("--json") => json = Some(value.clone()),
            Some("--made-for") => made_for = value.to_str().map(str::to_owned),
            _ => fail(format!("unknown option {}", option.display())),
        }
    }
    let surface = read_surface(surface);
    let sources = mod_uses::read_sources(dir, &[]).unwrap_or_else(|e| fail(e));
    let manifest = read_manifest_facts(dir).unwrap_or_else(|e| fail(e));
    let uses = uses_contained(
        entry,
        sources.iter().map(|(p, t)| (p.as_str(), t.as_str())),
        &manifest,
        &surface,
    )
    .unwrap_or_else(|e| fail(e));
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for used in &uses.items {
        let place = match &used.origin {
            Origin::Source { file, line } => format!("{file}:{line}"),
            Origin::Manifest => "sunrise-mod.toml".to_owned(),
            Origin::Via(id) => format!("via {id}"),
        };
        println!("{}  {}  {place}", used.entry.id, used.entry.hash);
        let kind = used.entry.id.split(':').next().unwrap_or_default();
        *kinds.entry(kind).or_default() += 1;
    }
    let counts: Vec<String> = kinds.iter().map(|(k, n)| format!("{k}={n}")).collect();
    println!("uses {} items: {}", uses.items.len(), counts.join(" "));
    for note in uses.items.iter().filter_map(|used| used.note.as_deref()) {
        println!("{note}");
    }
    if let Some(out) = json {
        let made_for = made_for.unwrap_or_else(|| fail("--json needs --made-for BUILD"));
        std::fs::write(&out, uses.table(made_for).to_json())
            .unwrap_or_else(|e| fail(format!("{}: {e}", out.display())));
    }
}

fn fits_command(args: &[PathBuf]) -> ! {
    let [table, target] = args else {
        fail("usage: sunrise-stage fits USES_JSON TARGET_SURFACE")
    };
    // Read at most one byte past the cap, so the reader refuses an oversized table by its size.
    let mut bytes = Vec::new();
    std::fs::File::open(table)
        .and_then(|file| {
            use std::io::Read;
            file.take(mod_surface::uses::MAX_USES_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .unwrap_or_else(|e| fail(format!("{}: {e}", table.display())));
    let table =
        UsesTable::read(&bytes).unwrap_or_else(|e| fail(format!("{}: {e}", table.display())));
    let target = read_surface(target);
    if table
        .entries
        .iter()
        .any(|entry| entry.id.starts_with("game:"))
    {
        fail(
            "the list records game code the mod declares, which a surface alone cannot judge: use `cargo sunrise-mod check --build <name>`"
                .to_owned(),
        );
    }
    let report = table.fits(&table.entries, &target);
    let code = match report.verdict {
        Fit::Fits => {
            println!(
                "fits ({} items, made for {})",
                table.entries.len(),
                table.made_for
            );
            0
        }
        Fit::Unversioned(hooks) => {
            eprintln!(
                "sunrise-stage: not known: nothing the list can judge changed, but it holds no version of {}; scan the mod again",
                hooks.join(", ")
            );
            2
        }
        Fit::DoesNotFit(misfits) => {
            println!(
                "does not fit: {} of the {} items it uses changed",
                misfits.len(),
                table.entries.len()
            );
            for misfit in &misfits {
                println!("  {misfit}  [{}]", misfit.id);
            }
            1
        }
    };
    if let Some(note) = report.deferred {
        println!("  {note}");
    }
    std::process::exit(code)
}

fn surface_diff(args: &[PathBuf]) -> ! {
    use sunrise_mod_api_gen::surface::notes;
    let [old, new, options @ ..] = args else {
        fail(
            "usage: sunrise-stage surface-diff OLD NEW [--notes DIR] [--base-notes DIR] [--api-notes-only] [--landed SHA] [--covered-by DIR --covered-base DIR] [--json OUT] [--gate]",
        )
    };
    let (mut notes_dir, mut base_notes, mut json, mut landed) = (None, None, None, None);
    let (mut covered_by, mut covered_base) = (None, None);
    let (mut gate, mut api_notes_only) = (false, false);
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let slot = match option.to_str() {
            Some("--gate") => {
                gate = true;
                continue;
            }
            Some("--api-notes-only") => {
                api_notes_only = true;
                continue;
            }
            Some("--notes") => &mut notes_dir,
            Some("--base-notes") => &mut base_notes,
            Some("--json") => &mut json,
            Some("--landed") => &mut landed,
            Some("--covered-by") => &mut covered_by,
            Some("--covered-base") => &mut covered_base,
            _ => fail(format!("unknown option {}", option.display())),
        };
        let value = options
            .next()
            .unwrap_or_else(|| fail(format!("{} needs a value", option.display())));
        *slot = Some(value);
    }
    let mut diff = mod_surface::diff(&read_surface(old), &read_surface(new));
    let filter_ids = |file: &mut notes::NoteFile| {
        if api_notes_only {
            file.ids
                .retain(|id| !matches!(id.split_once(':'), Some(("loop" | "internal", _))));
        }
    };
    let mut stale = Vec::new();
    if let Some(dir) = notes_dir {
        // Without the base's notes every older note would read as new, and stale.
        let base = base_notes.unwrap_or_else(|| fail("--notes needs --base-notes"));
        let found = notes::read(dir, base).unwrap_or_else(|e| fail(e));
        for mut file in found {
            // The notes of one merge are checked against that merge's diff, and no other's.
            let wanted = landed
                .and_then(|sha| sha.to_str())
                .map(str::to_ascii_lowercase);
            if file.landed != wanted {
                continue;
            }
            filter_ids(&mut file);
            for id in diff.attach(&file.note, &file.ids) {
                let hint = diff
                    .nearest(&id)
                    .map(|known| format!("; did you mean {known}?"))
                    .unwrap_or_default();
                stale.push(format!(
                    "change note {} names {id}, which did not change{hint}",
                    file.note.file
                ));
            }
        }
    }
    if let Some(dir) = covered_by {
        let base = covered_base.unwrap_or_else(|| fail("--covered-by needs --covered-base"));
        // Read apart from --notes, so neither set's file names can hide or replace the other's.
        for mut file in notes::read(dir, base).unwrap_or_else(|e| fail(e)) {
            if file.landed.is_none() {
                filter_ids(&mut file);
                diff.attach(&file.note, &file.ids);
            }
        }
    }
    print!("{}", diff.to_text());
    if let Some(out) = json {
        let text = serde_json::to_string_pretty(&diff).expect("a diff serializes") + "\n";
        std::fs::write(out, text).unwrap_or_else(|e| fail(format!("{}: {e}", out.display())));
    }
    let mut failed = !stale.is_empty();
    for line in stale {
        eprintln!("{line}");
    }
    if gate {
        for change in diff.undeclared() {
            eprintln!("breaking change without a change note: {}", change.id);
            failed = true;
        }
    }
    std::process::exit(i32::from(failed))
}

fn main() {
    // The uses scan parses in a child, this program started again: run its part and exit.
    let entry = mod_surface::child::child_entry();
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    match args.first().and_then(|c| c.to_str()) {
        Some("surface") => return surface(&args[1..]),
        Some("surface-catalogs") => return surface_catalogs(&args[1..]),
        Some("function-index") => return function_index(&args[1..]),
        Some("uses") => return uses_command(entry, &args[1..]),
        Some("fits") => fits_command(&args[1..]),
        Some("surface-diff") => surface_diff(&args[1..]),
        Some("surface-read") => surface_read(&args[1..]),
        _ => {}
    }
    let usage = || {
        eprintln!(
            "usage: sunrise-stage ROOT INTERNALS_JSON COMMIT [--sync-internals]\n       sunrise-stage surface ROOT OUT [--with-loops] [--with-internals]\n       sunrise-stage surface-catalogs ROOT SURFACE [--with-internals]\n       sunrise-stage surface-diff OLD NEW [--notes DIR] [--base-notes DIR] [--api-notes-only] [--landed SHA] [--covered-by DIR --covered-base DIR] [--json OUT] [--gate]\n       sunrise-stage surface-read IN OUT\n       sunrise-stage uses SURFACE MOD_DIR [--json OUT --made-for BUILD]\n       sunrise-stage fits USES_JSON TARGET_SURFACE"
        );
        std::process::exit(2)
    };
    let [root, internals, commit, options @ ..] = &args[..] else {
        usage()
    };
    // Discovery never builds or publishes a game.
    let mode = match options {
        [] => Mode::Build,
        [flag] if flag == "--sync-internals" => Mode::Discovery,
        _ => usage(),
    };
    let Some(commit) = commit.to_str() else {
        usage()
    };
    let build = staging::Build {
        commit: commit.to_owned(),
    };
    match SourceLayout::read(root)
        .and_then(|layout| staging::stage_with_layout(root, layout, internals, mode, &build))
    {
        Ok(staging::Staged {
            replace,
            internals: widened,
        }) => {
            println!(
                "gave {} functions a replacement slot in {} files of {} crates",
                replace.functions, replace.files, replace.crates
            );
            let shapes = widened
                .entries
                .iter()
                .filter(|entry| entry.shape_only)
                .count();
            println!(
                "exposed {} internals and listed {} type shapes in {} changed files: {:?}",
                widened.entries.len() - shapes,
                shapes,
                widened.files,
                widened.changed
            );
        }
        Err(error) => {
            eprintln!("sunrise-stage: {error}");
            std::process::exit(1);
        }
    }
}
