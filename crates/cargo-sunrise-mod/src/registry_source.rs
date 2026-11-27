//! Read the signed v2 registry index (sunrise-mods `docs/registry-v2.md`, "Reader rules") for
//! collection scaffolding and library mods. Before a build reads a library package, check its
//! size, SHA-256 and GitHub artifact attestation.
use crate::{
    bounded::{self, Symlinks},
    check::require_release_signer,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use sunrise_mod_package::index::{
    self,
    registry::builds::MAX_SIGNATURE,
    v2::{Index, Project, Release},
};

/// The registry's index key, published as `registry/index-key.pub` in sunrise-org/sunrise-mods.
/// Built in, never fetched: a registry that serves another key can't sign for this tool.
const KEY: &str = "RWTNJaFJ4AekHG/RgVrgmJpvy9B2EaeqbdSNuseKUx2XPxaOZkChLV0q";
/// The highest serial this computer accepted, in the tool's cache: an older index is refused.
const SERIAL: &str = "registry/v2-serial";

type Fetch = dyn Fn(&str, u64) -> Result<Vec<u8>, String>;
type Attest = dyn Fn(&[u8], [u8; 32], &Project, &Release) -> Result<(), String>;

pub(crate) struct Registry {
    pub(crate) index: Index,
    fetch: Box<Fetch>,
    attest: Box<Attest>,
}

impl Registry {
    pub(crate) fn read(cache: &Path) -> Result<Self, String> {
        let url = test_url().unwrap_or_else(|| sunrise_addresses::MOD_REGISTRY_INDEX.into());
        let index = read_index(cache, &url, &download, KEY)?;
        Ok(Self {
            index,
            fetch: Box::new(download),
            attest: Box::new(attest),
        })
    }

    /// A registry over a fixture index, with stand-ins for downloads and attestation checks.
    #[cfg(test)]
    pub(crate) fn fixture(
        index: Index,
        fetch: impl Fn(&str, u64) -> Result<Vec<u8>, String> + 'static,
        attest: impl Fn(&[u8], [u8; 32], &Project, &Release) -> Result<(), String> + 'static,
    ) -> Self {
        Self {
            index,
            fetch: Box::new(fetch),
            attest: Box::new(attest),
        }
    }

    /// The release's package, downloaded once into the cache and checked again on every use, so
    /// an edited cache entry is refused. The caller has already skipped taken-down releases.
    pub(crate) fn package(
        &self,
        cache: &Path,
        project: &Project,
        release: &Release,
    ) -> Result<PathBuf, String> {
        let failed = |why: String| {
            format!(
                "{} {} (registry project {}): {why}",
                project.mod_id, release.version, project.id
            )
        };
        let file = &release.file;
        let parent = cache.join("registry").join(&project.id);
        let kept = parent.join(&file.sha256);
        let (archive, bundle) = (
            kept.join("package.smod"),
            kept.join("package.sigstore.json"),
        );
        if !kept.is_dir() {
            fs::create_dir_all(&parent).map_err(|e| failed(e.to_string()))?;
            let pending = tempfile::tempdir_in(&parent).map_err(|e| failed(e.to_string()))?;
            for (url, limit, name) in [
                (&file.url, file.size, "package.smod"),
                (&file.attestation, index::MAX_FILE, "package.sigstore.json"),
            ] {
                let bytes = (self.fetch)(url, limit).map_err(failed)?;
                fs::write(pending.path().join(name), bytes).map_err(|e| failed(e.to_string()))?;
            }
            // A concurrent invocation may have kept the same download first; both are checked.
            if let Err(e) = fs::rename(pending.path(), &kept)
                && !kept.is_dir()
            {
                return Err(failed(e.to_string()));
            }
        }
        let bytes = bounded::read_bytes(&archive, file.size, Symlinks::Refuse)
            .map_err(|e| failed(e.into()))?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if bytes.len() as u64 != file.size || index::sha256(&bytes) != file.sha256 {
            return Err(failed(format!(
                "{} differs from the size and SHA-256 the index lists; delete the folder {}",
                archive.display(),
                kept.display()
            )));
        }
        let attestation = bounded::read_bytes(&bundle, index::MAX_FILE, Symlinks::Refuse)
            .map_err(|e| failed(e.into()))?;
        (self.attest)(&attestation, digest, project, release).map_err(failed)?;
        Ok(archive)
    }
}

fn read_index(cache: &Path, url: &str, fetch: &Fetch, key: &str) -> Result<Index, String> {
    let signed = fetch(url, index::MAX_INDEX + MAX_SIGNATURE as u64)?;
    let index = Index::verify(&signed, key).map_err(|e| format!("{url}: {e}"))?;
    let path = cache.join(SERIAL);
    let last = match fs::read_to_string(&path) {
        Ok(text) => Some(text.trim().parse::<u64>().map_err(|e| {
            format!(
                "{}: {e}; delete it to accept the live index",
                path.display()
            )
        })?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if let Some(last) = last
        && index.serial < last
    {
        return Err(format!(
            "{url}: serial {} is older than {last}, the last one this computer accepted",
            index.serial
        ));
    }
    if last.is_none_or(|last| index.serial > last) {
        // Replaced whole by a rename, so a stopped run never leaves a partial serial behind.
        let folder = path.parent().expect("serial file in the cache");
        fs::create_dir_all(folder)
            .and_then(|()| tempfile::NamedTempFile::new_in(folder))
            .and_then(|mut file| {
                file.write_all(index.serial.to_string().as_bytes())?;
                file.persist(&path).map_err(|e| e.error)?;
                Ok(())
            })
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(index)
}

/// The attestation must be GitHub's for this digest, from the project's registered repository and
/// the exact shared-workflow ref the index approved for this release.
fn attest(
    bundle: &[u8],
    digest: [u8; 32],
    project: &Project,
    release: &Release,
) -> Result<(), String> {
    let attested = sunrise_attestation::verify(
        bundle,
        digest,
        sunrise_attestation::PUBLIC_GOOD_TRUSTED_ROOT,
    )
    .map_err(|e| format!("attestation: {e}"))?;
    let attested = require_release_signer(attested, Some(project.repository.id))
        .map_err(|e| format!("attestation: {e}"))?;
    if attested.signer_workflow_ref != release.workflow {
        return Err(format!(
            "attestation: expected workflow {}, attestation names {}",
            release.workflow, attested.signer_workflow_ref
        ));
    }
    Ok(())
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    sunrise_proxy::get(url, &[], None)
        .map_err(|e| format!("{url}: {e}"))?
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{url}: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{url}: exceeds its {limit} byte limit"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::read_first_index;

fn test_url() -> Option<String> {
    // A test points the index at a local address without changing the published one.
    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("SUNRISE_MOD_REGISTRY_TEST_BASE") {
        return Some(format!("{}/index.signed", base.trim_end_matches('/')));
    }
    None
}

/// Write checked, package-relative paths to a private, fresh directory. `create_new` detects
/// collisions on case-insensitive filesystems as well as exact duplicates.
pub(crate) fn write_files(
    directory: &Path,
    files: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<[u8]>)>,
) -> Result<(), String> {
    for (name, bytes) in files {
        let name = name.as_ref();
        #[cfg(windows)]
        if name.split('/').any(|part| {
            // Win32 strips trailing spaces/dots and can treat DOS device names as files.
            let stem = part
                .split('.')
                .next()
                .unwrap_or(part)
                .trim_end()
                .to_ascii_uppercase();
            part.ends_with([' ', '.'])
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || ["COM", "LPT"].into_iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|n| {
                        matches!(
                            n,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                })
        }) {
            return Err(format!("{name}: cannot stage a Windows-aliased path"));
        }
        let file = directory.join(name);
        let write = || -> io::Result<()> {
            fs::create_dir_all(file.parent().expect("file in the staging directory"))?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)?
                .write_all(bytes.as_ref())
        };
        write().map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}
