//! `needs`, `missing-extras` and `extras`: what a launcher asks the preparer instead of linking it (docs/api/prepare-extras.md).

use crate::artifacts::{self, Inputs, Lock};
use crate::pyjson::{self, Value};
use crate::{check, exporters, Log};
use std::fmt;
use std::path::{Path, PathBuf};

/// The replies' `format`: a new field keeps it, a changed or removed one makes the next.
pub const FORMAT: i64 = 1;

/// The game's prepared-data cache (`sunrise_core::prepared_files::FILE_NAME`), stale once an extra is added.
pub const DATA_FILE: &str = "sunrise-data.bin";

/// Why `needs`, `missing-extras` or `extras` couldn't answer: its `kind` in the error reply, one per case.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// `research-lock.json` is missing or doesn't name a version.
    Lock(String),
    /// `extras` was asked to add to resources that aren't prepared.
    Unprepared(PathBuf),
    /// A file couldn't be read, linked or removed.
    Io(String),
}

impl Error {
    /// The `kind` in the error reply (docs/api/prepare-extras.md).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Lock(_) => "lock",
            Self::Unprepared(_) => "unprepared",
            Self::Io(_) => "io",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) | Self::Io(error) => f.write_str(error),
            Self::Unprepared(resources) => write!(
                f,
                "{} is not prepared: sunrise-prepare --prepare",
                resources.display()
            ),
        }
    }
}

/// What a preparation of `root` will need beyond the player's Minecraft folder.
#[derive(Debug, PartialEq, Eq)]
pub struct Needs {
    /// No verified import of the pinned jars is there to reuse, here or in a sibling version folder.
    pub server_jar: bool,
    /// The version ships no matching exporters, so preparing compiles them with a JDK.
    pub jdk: bool,
}

/// [`Needs`] for `root`. Reusing a sibling's import links it into `root`, as a preparation would.
pub fn needs(root: &Path, log: Log) -> Result<Needs, Error> {
    Lock::read(root).map_err(Error::Lock)?;
    Ok(Needs {
        server_jar: !artifacts::reuse_imported(root, log).map_err(Error::Io)?,
        jdk: !exporters::available(root),
    })
}

/// The kinds `extras` adds, in the order it adds them.
pub fn kinds() -> impl Iterator<Item = &'static str> {
    check::EXTRAS.iter().map(|(_, _, kind)| *kind)
}

/// A resources folder and the extras it lacks, as `missing-extras` reports them.
#[derive(Debug, PartialEq, Eq)]
pub struct Missing {
    pub resources: PathBuf,
    /// It holds a preparation (`sunrise-resources.json`).
    pub prepared: bool,
    /// Each lacking extra's kind and what it is, in the order `extras` adds them.
    pub extras: Vec<(&'static str, &'static str)>,
}

/// What the resources of `root` (or `output`) lack.
pub fn missing(root: &Path, output: Option<&Path>) -> Result<Missing, Error> {
    let lock = Lock::read(root).map_err(Error::Lock)?;
    let resources = crate::resources_dir(root, &lock.version, output);
    Ok(Missing {
        prepared: resources.join("sunrise-resources.json").is_file(),
        extras: check::missing_extras(&resources),
        resources,
    })
}

/// Writes the prepared resources' `sunrise-data.bin` when they have none, so the game's first start reads one file: whether it wrote one.
pub fn data_file(root: &Path, output: Option<&Path>) -> Result<bool, Error> {
    let found = missing(root, output)?;
    if !found.prepared {
        return Err(Error::Unprepared(found.resources));
    }
    let data = found.resources.join(DATA_FILE);
    if data.exists() {
        return Ok(false);
    }
    let readers = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    sunrise_core::prepared_files::write_with_readers(&found.resources, readers)
        .map_err(|e| Error::Io(format!("{}: {e}", data.display())))?;
    Ok(true)
}

/// `data-file --json`'s reply.
pub fn data_file_json(written: bool) -> String {
    reply(vec![("written", Value::Bool(written))])
}

/// What `extras` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Added {
    pub added: Vec<&'static str>,
    /// Each extra no folder could give, with the first folder's reason.
    pub failed: Vec<(&'static str, String)>,
    /// What the resources still lack afterwards, of every kind.
    pub missing: Vec<(&'static str, &'static str)>,
}

/// Adds the extras the resources lack (or those of `only`), each from the first of `folders` that has it. No
/// folders add nothing, as the launcher's own completion does: there is no default folder.
pub fn add(
    root: &Path,
    output: Option<&Path>,
    only: Option<&[String]>,
    folders: &[PathBuf],
    inputs: &Inputs,
    log: Log,
) -> Result<Added, Error> {
    let Missing {
        resources,
        prepared,
        extras: lacking,
    } = missing(root, output)?;
    if !prepared {
        return Err(Error::Unprepared(resources));
    }
    let mut done = Added::default();
    for (kind, _) in lacking {
        if folders.is_empty() || only.is_some_and(|only| !only.iter().any(|k| k == kind)) {
            continue;
        }
        let mut first_error = None;
        let added = folders.iter().any(|folder| {
            crate::add_assets(kind, root, output, Some(folder.as_path()), inputs, log)
                .map_err(|e| first_error.get_or_insert(e))
                .is_ok()
        });
        match added {
            true => done.added.push(kind),
            false => done.failed.push((kind, first_error.unwrap_or_default())),
        }
    }
    done.missing = check::missing_extras(&resources);
    Ok(done)
}

fn reply(fields: Vec<(&str, Value)>) -> String {
    let mut object = vec![("format".to_owned(), Value::Int(FORMAT.to_string()))];
    object.extend(fields.into_iter().map(|(k, v)| (k.to_owned(), v)));
    pyjson::dumps(&Value::Object(object), &pyjson::DEFAULT)
}

fn extras(list: &[(&'static str, &'static str)]) -> Value {
    Value::Array(
        list.iter()
            .map(|(kind, what)| {
                Value::Object(vec![
                    ("kind".into(), Value::str(*kind)),
                    ("what".into(), Value::str(*what)),
                ])
            })
            .collect(),
    )
}

/// `needs --json`'s reply.
pub fn needs_json(needs: &Needs) -> String {
    reply(vec![
        ("server_jar", Value::Bool(needs.server_jar)),
        ("jdk", Value::Bool(needs.jdk)),
    ])
}

/// `missing-extras --json`'s reply.
pub fn missing_json(missing: &Missing) -> String {
    reply(vec![
        (
            "resources",
            Value::str(missing.resources.display().to_string()),
        ),
        ("prepared", Value::Bool(missing.prepared)),
        ("missing", extras(&missing.extras)),
    ])
}

/// `extras --json`'s last line.
pub fn added_json(added: &Added) -> String {
    let failed = added
        .failed
        .iter()
        .map(|(kind, error)| {
            Value::Object(vec![
                ("kind".into(), Value::str(*kind)),
                ("error".into(), Value::str(error.as_str())),
            ])
        })
        .collect();
    reply(vec![
        (
            "added",
            Value::Array(added.added.iter().map(|k| Value::str(*k)).collect()),
        ),
        ("failed", Value::Array(failed)),
        ("missing", extras(&added.missing)),
    ])
}

/// The reply of any of the three when it couldn't answer.
pub fn error_json(error: &Error) -> String {
    reply(vec![(
        "error",
        Value::Object(vec![
            ("kind".into(), Value::str(error.kind())),
            ("message".into(), Value::str(error.to_string())),
        ]),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_file_is_the_one_the_game_reads() {
        assert_eq!(DATA_FILE, sunrise_core::prepared_files::FILE_NAME);
    }

    #[test]
    fn replies_keep_their_shape() {
        assert_eq!(
            needs_json(&Needs {
                server_jar: true,
                jdk: false
            }),
            r#"{"format": 1, "server_jar": true, "jdk": false}"#
        );
        assert_eq!(
            missing_json(&Missing {
                resources: "/r".into(),
                prepared: true,
                extras: vec![("sounds", "the sounds")],
            }),
            r#"{"format": 1, "resources": "/r", "prepared": true, "missing": [{"kind": "sounds", "what": "the sounds"}]}"#
        );
        let added = Added {
            added: vec!["packs"],
            failed: vec![("sounds", "no index".into())],
            missing: vec![("sounds", "the sounds")],
        };
        assert_eq!(
            added_json(&added),
            r#"{"format": 1, "added": ["packs"], "failed": [{"kind": "sounds", "error": "no index"}], "missing": [{"kind": "sounds", "what": "the sounds"}]}"#
        );
        assert_eq!(
            error_json(&Error::Lock("no lock".into())),
            r#"{"format": 1, "error": {"kind": "lock", "message": "no lock"}}"#
        );
    }

    #[test]
    fn unprepared_resources_are_refused_and_report_every_extra() {
        let root = std::env::temp_dir().join(format!("sunrise-extras-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("research-lock.json"),
            r#"{"minecraft": {"version": "9.9", "java_major": 25}}"#,
        )
        .unwrap();
        let found = missing(&root, None).unwrap();
        assert_eq!(found.resources, root.join(".local/sunrise-client/9.9"));
        assert!(!found.prepared);
        let kinds: Vec<_> = found.extras.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, ["packs", "panorama", "fonts", "languages", "sounds"]);
        let error = add(&root, None, None, &[], &Inputs::default(), &|_| {}).unwrap_err();
        assert_eq!(error, Error::Unprepared(found.resources));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_root_without_a_lock_is_the_lock_kind() {
        let root =
            std::env::temp_dir().join(format!("sunrise-extras-nolock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for error in [
            missing(&root, None).unwrap_err(),
            needs(&root, &|_| {}).unwrap_err(),
            add(&root, None, None, &[], &Inputs::default(), &|_| {}).unwrap_err(),
        ] {
            assert_eq!(error.kind(), "lock", "{error}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_data_file_is_written_once_for_prepared_resources_only() {
        let root = std::env::temp_dir().join(format!("sunrise-extras-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let resources = root.join(".local/sunrise-client/9.9");
        std::fs::create_dir_all(resources.join("data/minecraft")).unwrap();
        std::fs::write(
            root.join("research-lock.json"),
            r#"{"minecraft": {"version": "9.9", "java_major": 25}}"#,
        )
        .unwrap();
        assert!(matches!(data_file(&root, None), Err(Error::Unprepared(_))));
        std::fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
        std::fs::write(resources.join("data/minecraft/a.json"), "{}").unwrap();
        assert_eq!(data_file(&root, None), Ok(true));
        assert!(resources.join(DATA_FILE).is_file());
        assert_eq!(
            data_file(&root, None),
            Ok(false),
            "one already there is kept"
        );
        assert_eq!(data_file_json(true), r#"{"format": 1, "written": true}"#);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_folders_add_nothing_and_leave_every_extra_missing() {
        let root =
            std::env::temp_dir().join(format!("sunrise-extras-nofolders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let resources = root.join(".local/sunrise-client/9.9");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(
            root.join("research-lock.json"),
            r#"{"minecraft": {"version": "9.9", "java_major": 25}}"#,
        )
        .unwrap();
        std::fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
        std::fs::write(resources.join(DATA_FILE), "cache").unwrap();
        let done = add(&root, None, None, &[], &Inputs::default(), &|_| {}).unwrap();
        assert_eq!((done.added.len(), done.failed.len()), (0, 0));
        let kinds: Vec<_> = done.missing.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, ["packs", "panorama", "fonts", "languages", "sounds"]);
        assert!(
            resources.join(DATA_FILE).is_file(),
            "nothing added keeps the cache"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
