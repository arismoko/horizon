//! One source staging pass for the launcher and `horizon-stage`.
use crate::{internals, points, replace};
use std::{path::Path, time::Instant};

/// Discovery instrumentation must never be used to build or publish a game.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Widen visibility, mark injection points and give functions their replacement slots,
    /// without diagnostic attributes.
    Build,
    /// Add internal-use diagnostics for an isolated metadata-only mod check.
    Discovery,
}

/// Counts and catalog produced by source staging.
pub struct Staged {
    /// Injection point counts.
    pub points: points::Staged,
    /// Replacement slot counts.
    pub replace: replace::Staged,
    /// Exposed internal signatures and changed visibility counts.
    pub internals: internals::Widened,
}

/// Expose internals, mark points, give functions their replacement slots and write the manifests
/// in a disposable source copy. Internals precede points so their catalog contains only the
/// original game items, and points precede the slots so only game functions are points.
/// A pass stamps the copy before edits; even a failed pass requires fresh source for a retry.
pub fn stage(
    root: &Path,
    points_json: &Path,
    internals_json: &Path,
    mode: Mode,
    api_version: &str,
) -> Result<Staged, String> {
    stage_with_catalog(root, points_json, internals_json, mode, || {
        crate::sdk_catalog::sdk_catalog(root, api_version)
    })
}

fn stage_with_catalog(
    root: &Path,
    points_json: &Path,
    internals_json: &Path,
    mode: Mode,
    catalog: impl FnOnce() -> Result<String, String>,
) -> Result<Staged, String> {
    let stamp = root.join(".horizon-source-staged");
    if stamp.exists() {
        return Err("source tree is already staged; use a fresh copy of the source".into());
    }
    std::fs::write(
        &stamp,
        "Source staging started. Use a fresh copy to stage again.\n",
    )
    .map_err(|error| format!("{}: {error}", stamp.display()))?;
    if matches!(mode, Mode::Build) {
        let started = Instant::now();
        let catalog = catalog()?;
        let destination = points_json.with_file_name(point_shape::SDK_CATALOG_FILE);
        std::fs::write(&destination, catalog)
            .map_err(|e| format!("{}: {e}", destination.display()))?;
        eprintln!(
            "stage_phase=catalog seconds={:.3}",
            started.elapsed().as_secs_f64()
        );
    }
    // Listed before any pass rewrites the source: the markers are the source's.
    let started = Instant::now();
    write(
        &points_json.with_file_name(crate::loops::FILE),
        &crate::loops::collect(root)?,
    )?;
    eprintln!(
        "stage_phase=loops seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let mut internals = match mode {
        Mode::Build => internals::widen(root)?,
        Mode::Discovery => internals::discover(root)?,
    };
    internals
        .entries
        .extend(crate::crossing::foreign_internals());
    write(internals_json, &internals.entries)?;
    eprintln!(
        "stage_phase=internals seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let wire = crate::build::wire_types(root)?;
    eprintln!(
        "stage_phase=wire seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let (points, records) = points::stage_with_manifest(root, &wire)?;
    eprintln!(
        "stage_phase=points seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    #[derive(serde::Serialize)]
    struct Manifest {
        points: Vec<points::GamePoint>,
        /// The `mod_api` types with a `Wire` layout: what an engine point's API record may hold.
        wire: std::collections::BTreeSet<String>,
    }
    write(
        points_json,
        &Manifest {
            points: records,
            wire,
        },
    )?;
    eprintln!(
        "stage_phase=point_manifest seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let replace = replace::stage(root)?;
    eprintln!(
        "stage_phase=replace seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    Ok(Staged {
        points,
        replace,
        internals,
    })
}

fn write(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Point staging fixtures isolate source rewriting from full hook resolution.
    fn stage(root: &Path, points: &Path, internals: &Path, mode: Mode) -> Result<Staged, String> {
        stage_with_catalog(root, points, internals, mode, || Ok("{}\n".into()))
    }

    #[test]
    fn staging_marks_points_and_slots_and_refuses_reuse() {
        // Each iteration starts from fresh source, not the preceding staged fixture.
        for copy in 0..3 {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().to_path_buf();
            let source = root.join("crates/horizon-client/src/game.rs");
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
                root.join("crates/horizon-client/Cargo.toml"),
                "[package]\nname = 'horizon-client'\n",
            )
            .unwrap();
            std::fs::write(
                root.join("Cargo.lock"),
                "version = 4\n\n[[package]]\nname = \"horizon-client\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
            let lists = root.join("crates/horizon-mod-api-gen");
            std::fs::create_dir_all(&lists).unwrap();
            std::fs::write(lists.join("internals.txt"), "").unwrap();
            let package = root.join("crates/horizon-mod-points");
            std::fs::create_dir_all(&package).unwrap();
            std::fs::write(
                package.join("Cargo.toml"),
                "[package]\nname = 'horizon-mod-points'\n",
            )
            .unwrap();
            let output = root.join("points.json");
            if copy == 2 {
                // Refuse retries even when a late output failure leaves edited source.
                std::fs::create_dir(&output).unwrap();
                assert!(stage(&root, &output, &root.join("internals.json"), Mode::Build).is_err());
            } else {
                stage(&root, &output, &root.join("internals.json"), Mode::Build).unwrap();
                assert_eq!(
                    std::fs::read_to_string(root.join(point_shape::SDK_CATALOG_FILE)).unwrap(),
                    "{}\n"
                );
                let bytes = std::fs::read(&output).unwrap();
                assert_eq!(bytes.last(), Some(&b'\n'));
                let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let points = manifest["points"].as_array().unwrap();
                for (name, returns) in [("unit", "cancel"), ("optional", "result")] {
                    let point = points.iter().find(|point| point["name"] == name).unwrap();
                    assert_eq!(point["returns"], returns);
                }
            }
            let before = std::fs::read(&source).unwrap();
            let text = String::from_utf8_lossy(&before);
            assert!(text.contains("horizon_mod_api_macros"));
            // The slots follow the points: a pass that failed between them left none.
            assert_eq!(
                text.contains("slot = 0") && text.contains("__fnr_unit"),
                copy != 2
            );
            for mode in [Mode::Build, Mode::Discovery] {
                let error = stage(&root, &output, &root.join("internals.json"), mode)
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
}
