//! Reusable, immutable mixin output. API packages are staged after this snapshot.
use super::{Bundle, Profile};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

/// Hash all mixin inputs but not an API crate that mixins do not read.
pub(super) fn key(
    revision: &str,
    mixins: &str,
    profile: Profile,
    mods: &[Bundle],
) -> Result<String, String> {
    let mut hash = Sha256::new();
    for part in [
        // Bump when launcher generation or validation changes the snapshot's contents.
        "flat-trait-injection-v1",
        super::STAGING,
        revision,
        mixins,
        // The Cargo profile: `Performance` and `Dev` stage the same source.
        profile.cargo().0,
        super::rust_toolchain::VERSION,
    ] {
        field(&mut hash, part.as_bytes());
    }
    for bundle in mods {
        field(&mut hash, b"bundle");
        field(&mut hash, bundle.manifest.id.as_bytes());
        // Trait-only edits do not change the patched source. A raw source helper may
        // include other crate files, so hash that crate conservatively when one is used.
        let reads_source = bundle
            .native()
            .injection
            .iter()
            .any(|entry| entry.helper.is_some());
        for (name, path) in super::bundle_files(bundle)? {
            if !reads_source
                && (name == "Cargo.toml" || name == "build.rs" || name.starts_with("src/"))
            {
                continue;
            }
            field(&mut hash, b"file");
            field(&mut hash, name.as_bytes());
            field(
                &mut hash,
                &fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            );
        }
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub(super) fn snapshot(root: &Path, key: &str) -> PathBuf {
    root.join("patched").join(key)
}

/// Record a hit and retain the three most recently used snapshots.
pub(super) fn touch_and_prune(root: &Path, key: &str) -> Result<(), String> {
    let dir = root.join("patched");
    fs::write(dir.join(key).join(".last-used"), b"").map_err(|e| e.to_string())?;
    let mut entries = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| {
        std::cmp::Reverse((
            e.file_name() == key,
            fs::metadata(e.path().join(".last-used"))
                .and_then(|m| m.modified())
                .ok(),
        ))
    });
    for entry in entries.into_iter().skip(3) {
        fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
