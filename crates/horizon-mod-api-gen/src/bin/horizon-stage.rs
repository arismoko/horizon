//! `horizon-stage ROOT POINTS_JSON INTERNALS_JSON API_VERSION`: expose internals, mark points and give functions their replacement slots in a staged
//! source copy, then write both manifests (#589). Never run on a working tree.
//!
//! `horizon-stage surface ROOT OUT`: write the surface export (`surface.json`) of the source tree
//! at ROOT to OUT, and nothing else. It only reads the tree, so a working tree is fine.
//!
//! `horizon-stage surface-diff OLD NEW [--notes DIR] [--base-notes DIR] [--json OUT] [--gate]`:
//! print what changed between two surface exports, breaking changes first. `--notes` attaches
//! the change notes of DIR (`crates/horizon-mod-vocab/changes`) whose file names are not in
//! `--base-notes` (the base tree's folder; required with `--notes`). Exits 1 when a new note
//! names an id that did not change, or with `--gate` when a breaking change has no note; 2 when
//! an input cannot be read or the options are wrong.
use horizon_mod_api_gen::staging::{self, Mode};
use mod_surface::Surface;
use std::path::PathBuf;

fn surface(root: &std::path::Path, out: &std::path::Path) {
    let started = std::time::Instant::now();
    // The generator writes source paths into `#[path]`s it then reads: they must be absolute.
    let root = match std::fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("horizon-stage surface: {}: {error}", root.display());
            std::process::exit(1);
        }
    };
    let export = match horizon_mod_api_gen::surface::export(&root) {
        Ok(export) => export,
        Err(error) => {
            eprintln!("horizon-stage surface: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = std::fs::write(out, export.surface.to_json()) {
        eprintln!("horizon-stage surface: {}: {error}", out.display());
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
    eprintln!("horizon-stage: {message}");
    std::process::exit(2)
}

fn read_surface(path: &std::path::Path) -> Surface {
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    Surface::read(&bytes).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
}

fn surface_diff(args: &[PathBuf]) -> ! {
    use horizon_mod_api_gen::surface::notes;
    let [old, new, options @ ..] = args else {
        fail(
            "usage: horizon-stage surface-diff OLD NEW [--notes DIR] [--base-notes DIR] [--json OUT] [--gate]",
        )
    };
    let (mut notes_dir, mut base_notes, mut json, mut gate) = (None, None, None, false);
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let slot = match option.to_str() {
            Some("--gate") => {
                gate = true;
                continue;
            }
            Some("--notes") => &mut notes_dir,
            Some("--base-notes") => &mut base_notes,
            Some("--json") => &mut json,
            _ => fail(format!("unknown option {}", option.display())),
        };
        let value = options
            .next()
            .unwrap_or_else(|| fail(format!("{} needs a value", option.display())));
        *slot = Some(value);
    }
    let mut diff = mod_surface::diff(&read_surface(old), &read_surface(new));
    let mut stale = Vec::new();
    if let Some(dir) = notes_dir {
        // Without the base's notes every older note would read as new, and stale.
        let base = base_notes.unwrap_or_else(|| fail("--notes needs --base-notes"));
        let found = notes::read(dir, base).unwrap_or_else(|e| fail(e));
        for file in found {
            for id in diff.attach(&file.note, &file.ids) {
                stale.push(format!(
                    "change note {} names {id}, which did not change",
                    file.note.file
                ));
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
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.first().and_then(|c| c.to_str()) == Some("surface-diff") {
        surface_diff(&args[1..]);
    }
    if let [command, root, out] = &args[..]
        && command == "surface"
    {
        return surface(root, out);
    }
    let usage = || {
        eprintln!(
            "usage: horizon-stage ROOT POINTS_JSON INTERNALS_JSON API_VERSION [--sync-internals]\n       horizon-stage surface ROOT OUT\n       horizon-stage surface-diff OLD NEW [--notes DIR] [--base-notes DIR] [--json OUT] [--gate]"
        );
        std::process::exit(2)
    };
    let [root, list, internals, api_version, options @ ..] = &args[..] else {
        usage()
    };
    // Discovery never builds or publishes a game.
    let mode = match options {
        [] => Mode::Build,
        [flag] if flag == "--sync-internals" => Mode::Discovery,
        _ => usage(),
    };
    let Some(api_version) = api_version.to_str() else {
        usage()
    };
    match staging::stage(root, list, internals, mode, api_version) {
        Ok(staging::Staged {
            points: staged,
            replace,
            internals: widened,
        }) => {
            println!(
                "staged {} points ({} with a head only; {} functions have no API record) in {} files",
                staged.points, staged.head_only, staged.refused, staged.files
            );
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
            eprintln!("horizon-stage: {error}");
            std::process::exit(1);
        }
    }
}
