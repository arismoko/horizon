//! Checked prebuilt metadata, shared by the launcher and mod tools.
use super::ListedInternal;
use cap_std::fs::Dir;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

use crate::build_data::{INTERNALS as CATALOG, POINTS};
const CATALOG_LIMIT: u64 = 64 << 20;
const ARCHIVE_LIMIT: u64 = 4 << 30;

/// Why a prebuilt metadata archive could not be cached.
#[derive(Debug)]
#[non_exhaustive]
pub enum CatalogError {
    /// Reading or writing the archive failed.
    Io(std::io::Error),
    /// A build-data file is not the expected form, or has an unknown schema.
    BuildData(String),
    /// The compressed archive exceeded its download bound.
    ArchiveTooLarge { archive: String },
    /// The archive did not match its published checksum.
    Checksum { archive: String },
    /// A required root metadata file was absent.
    Missing { file: &'static str },
    /// A root metadata file appeared more than once.
    Repeated { file: &'static str },
    /// A metadata member was not a regular file.
    NotFile { file: &'static str },
    /// A metadata file exceeded its decoded size bound.
    CatalogTooLarge { file: &'static str },
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::BuildData(why) => f.write_str(why),
            Self::ArchiveTooLarge { archive } => {
                write!(f, "{archive}: prebuilt archive exceeds 4 GiB")
            }
            Self::Checksum { archive } => write!(
                f,
                "The prebuilt game {archive} doesn't match its release's checksum. Try again."
            ),
            Self::Missing { file } => write!(f, "prebuilt archive has no {file}"),
            Self::Repeated { file } => write!(f, "{file}: repeated file in the prebuilt archive"),
            Self::NotFile { file } => write!(f, "{file}: not a file in the prebuilt archive"),
            Self::CatalogTooLarge { file } => write!(f, "{file}: catalog exceeds 64 MiB"),
        }
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CatalogError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Verify a prebuilt archive and cache only its root internals and points JSON files.
///
/// The caller owns transport and the cache directory capability. Both files are checked
/// before either is published; no game code or other archive member is unpacked.
/// Existing internals declarations are returned for the mod tools' signature comparison.
/// `name` is diagnostic text, never a filesystem path.
pub fn cache_catalogs(
    cache: &Dir,
    name: &str,
    body: impl Read,
    expected: &str,
) -> Result<Vec<ListedInternal>, CatalogError> {
    cache_with_limit(cache, name, body, expected, ARCHIVE_LIMIT)
}

fn cache_with_limit(
    cache: &Dir,
    name: &str,
    body: impl Read,
    expected: &str,
    limit: u64,
) -> Result<Vec<ListedInternal>, CatalogError> {
    let pid = std::process::id();
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let unique = format!("{pid}.{serial}");
    let archive = format!("prebuilt.{unique}.partial");
    let result = (|| {
        let mut body = body.take(limit + 1);
        let mut file = cache.create(&archive)?;
        let mut hash = Sha256::new();
        let mut size = 0;
        let mut buffer = [0; 65536];
        loop {
            let read = body.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            size += read as u64;
            if size > limit {
                return Err(CatalogError::ArchiveTooLarge {
                    archive: name.into(),
                });
            }
            hash.update(&buffer[..read]);
            file.write_all(&buffer[..read])?;
        }
        drop(file);
        if format!("{:x}", hash.finalize()) != expected {
            return Err(CatalogError::Checksum {
                archive: name.into(),
            });
        }
        let catalogs = catalogs_from_archive(cache.open(&archive)?)?;
        let entries = super::parse(&catalogs.internals).map_err(CatalogError::BuildData)?;
        crate::build_data::read::<serde_json::Map<String, serde_json::Value>>(
            POINTS,
            &catalogs.points,
        )
        .map_err(CatalogError::BuildData)?;
        for (name, text) in [(CATALOG, catalogs.internals), (POINTS, catalogs.points)] {
            let partial = format!("{name}.{unique}.partial");
            cache.write(&partial, text)?;
            cache.rename(&partial, cache, name)?;
        }
        Ok(entries)
    })();
    let _ = cache.remove_file(&archive);
    for name in [CATALOG, POINTS] {
        let _ = cache.remove_file(format!("{name}.{unique}.partial"));
    }
    result
}

struct Catalogs {
    internals: String,
    points: String,
}

fn catalogs_from_archive(reader: impl Read) -> Result<Catalogs, CatalogError> {
    let reader = flate2::read::GzDecoder::new(reader).take(8 << 30);
    let mut archive = tar::Archive::new(reader);
    let (mut internals, mut points) = (None, None);
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        let (name, target) = match path.as_ref() {
            p if p == Path::new(CATALOG) || p == Path::new("./internals.json") => {
                (CATALOG, &mut internals)
            }
            p if p == Path::new(POINTS) || p == Path::new("./points.json") => (POINTS, &mut points),
            _ => continue,
        };
        if target.is_some() {
            return Err(CatalogError::Repeated { file: name });
        }
        if !entry.header().entry_type().is_file() {
            return Err(CatalogError::NotFile { file: name });
        }
        let mut text = String::new();
        entry.take(CATALOG_LIMIT + 1).read_to_string(&mut text)?;
        if text.len() as u64 > CATALOG_LIMIT {
            return Err(CatalogError::CatalogTooLarge { file: name });
        }
        *target = Some(text);
    }
    Ok(Catalogs {
        internals: internals.ok_or(CatalogError::Missing { file: CATALOG })?,
        points: points.ok_or(CatalogError::Missing { file: POINTS })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const JSON: &str = r#"{"schema":1,"build":{"commit":"c"},"internals":[{"path":"Example","kind":"struct","signature":"struct Example","hash":"abc","compiler_owned":{"ref":"Example::field"}}]}"#;
    const PTS: &str = r#"{"schema":1,"build":{"commit":"c"},"points":[]}"#;

    fn archive(path: &str, kind: tar::EntryType, contents: &str, points: Option<&str>) -> Vec<u8> {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(gzip);
        let unrelated = ("bin/game", tar::EntryType::Regular, "not code");
        for (path, kind, contents) in std::iter::once((path, kind, contents))
            .chain(points.map(|text| (POINTS, tar::EntryType::Regular, text)))
            .chain(std::iter::once(unrelated))
        {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(kind);
            header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            archive.append(&header, contents.as_bytes()).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn reads_only_regular_root_catalogs() {
        let bytes = archive("./internals.json", tar::EntryType::Regular, JSON, Some(PTS));
        let found = catalogs_from_archive(bytes.as_slice()).unwrap();
        assert_eq!(found.internals, JSON);
        assert_eq!(found.points, PTS);
        for path in [
            "deps/internals.json",
            "../internals.json",
            "/internals.json",
        ] {
            let bytes = archive(path, tar::EntryType::Regular, JSON, Some(PTS));
            assert!(matches!(
                catalogs_from_archive(bytes.as_slice()),
                Err(CatalogError::Missing { file: CATALOG })
            ));
        }
        let mut oversized = format!("[{}]", " ".repeat((64 << 20) - 1));
        let bytes = archive(CATALOG, tar::EntryType::Regular, &oversized, Some(PTS));
        assert!(matches!(
            catalogs_from_archive(bytes.as_slice()),
            Err(CatalogError::CatalogTooLarge { file: CATALOG })
        ));
        oversized.remove(1);
        let bytes = archive(CATALOG, tar::EntryType::Regular, &oversized, Some(PTS));
        assert!(catalogs_from_archive(bytes.as_slice()).is_ok());
        for (bytes, expected) in [
            (
                archive(CATALOG, tar::EntryType::Symlink, JSON, Some(PTS)),
                "not a file",
            ),
            (
                archive(CATALOG, tar::EntryType::Regular, JSON, None),
                "no points.json",
            ),
            (
                archive(POINTS, tar::EntryType::Regular, PTS, Some(PTS)),
                "repeated file",
            ),
        ] {
            assert!(
                catalogs_from_archive(bytes.as_slice())
                    .err()
                    .unwrap()
                    .to_string()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn cached_only_after_checksum_and_both_json_checks() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/catalog-cache-tests")
            .join(std::process::id().to_string());
        std::fs::create_dir_all(&path).unwrap();
        let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let mut bytes = archive(CATALOG, tar::EntryType::Regular, JSON, Some(PTS));
        bytes.resize(4 * 1024 + 1, 0);
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let mut reader = std::io::Cursor::new(bytes.as_slice());
        assert!(matches!(
            cache_with_limit(&dir, "game.tar.gz", &mut reader, &hash, ARCHIVE_LIMIT >> 20),
            Err(CatalogError::ArchiveTooLarge { .. })
        ));
        assert_eq!(reader.position(), 4 * 1024 + 1);
        let bytes = archive(CATALOG, tar::EntryType::Regular, JSON, Some(PTS));
        assert!(
            cache_catalogs(&dir, "game.tar.gz", bytes.as_slice(), "wrong")
                .unwrap_err()
                .to_string()
                .contains("checksum")
        );
        let headless =
            r#"[{"path":"Example","kind":"struct","signature":"struct Example","hash":"abc"}]"#;
        let later = JSON.replace(r#""schema":1"#, r#""schema":2"#);
        for (text, points) in [
            ("not JSON", PTS),
            ("{}", PTS),
            (headless, PTS),
            (later.as_str(), PTS),
            (JSON, "not JSON"),
            (JSON, "{}"),
        ] {
            let invalid = archive(CATALOG, tar::EntryType::Regular, text, Some(points));
            let hash = format!("{:x}", Sha256::digest(&invalid));
            assert!(cache_catalogs(&dir, "game.tar.gz", invalid.as_slice(), &hash).is_err());
        }
        assert_eq!(dir.entries().unwrap().count(), 0);
        let outside = path.with_extension("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), "protected").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, path.join("diagnostic-link")).unwrap();
        let diagnostic = if cfg!(unix) {
            path.join("diagnostic-link/sentinel")
        } else {
            outside.join("sentinel")
        };
        let hash = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(
            cache_catalogs(&dir, diagnostic.to_str().unwrap(), bytes.as_slice(), &hash).unwrap()[0]
                .path,
            "Example"
        );
        assert_eq!(dir.read_to_string(CATALOG).unwrap(), JSON);
        assert_eq!(dir.read_to_string(POINTS).unwrap(), PTS);
        assert_eq!(
            std::fs::read_to_string(outside.join("sentinel")).unwrap(),
            "protected"
        );
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        assert_eq!(
            dir.entries().unwrap().count(),
            if cfg!(unix) { 3 } else { 2 }
        );
        let invalid = archive(CATALOG, tar::EntryType::Regular, JSON, Some("not JSON"));
        let hash = format!("{:x}", Sha256::digest(&invalid));
        assert!(cache_catalogs(&dir, "game.tar.gz", invalid.as_slice(), &hash).is_err());
        assert_eq!(dir.read_to_string(CATALOG).unwrap(), JSON);
        assert_eq!(dir.read_to_string(POINTS).unwrap(), PTS);
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
}
