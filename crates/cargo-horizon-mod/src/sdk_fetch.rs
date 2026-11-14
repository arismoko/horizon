//! The pinned SDK in a mod project's `.horizon/sdk/`: what `horizon-mod-api`'s path dependency
//! (`.horizon/sdk/api/crates/horizon-mod-api`) points at, fetched by every command that needs it
//! when it is missing or another build's. Only a verified release lands there: the release's
//! `SHA256SUMS` must carry Horizon's signature for the tag `sdk-<build id>`, `sdk.json` must be the
//! file those sums list and describe that build, and every other file must match `sdk.json`.
//! `HORIZON_SDK_BASE` (a URL or a folder) replaces the releases' base for a mirror or offline use;
//! the checks are the same.
use horizon_mod_package::{
    Manifest,
    index::native,
    manifest::MANIFEST,
    sdk::{self, Index},
};
use horizon_release_sign::PublicKey;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The SDK's folder in a project.
pub(crate) const DIR: &str = ".horizon/sdk";
/// The releases' base, when `HORIZON_SDK_BASE` names none.
const BASE: &str = horizon_addresses::SDK_RELEASES;
/// The file naming the build a folder holds, written last: a folder without it is not one.
const STAMP: &str = "build";
const SURFACE: &str = "surface.json";
const API_SOURCE: &str = "api-source.tar.xz";
/// The most a small file (`sdk.json`, the sums, the signature, `surface.json`) may be.
const SMALL: u64 = 16 << 20;
/// The most the API source may unpack to.
const API_LIMIT: u64 = 512 << 20;

/// A fetched, verified SDK.
#[derive(Debug)]
pub(crate) struct PinnedSdk {
    /// The build id: the game's full commit sha.
    pub(crate) build: String,
    /// The form people read: `nightly-2026-11-13 (afe471c)`, or the short sha.
    pub(crate) name: String,
    pub(crate) dir: PathBuf,
    /// The build's surface export, `surface.json` in `dir`.
    pub(crate) surface: PathBuf,
}

/// The SDK the project at `project` pins, in its `.horizon/sdk/`, fetched first when the folder is
/// missing or holds another build.
pub(crate) fn ensure_sdk(project: &Path) -> Result<PinnedSdk, String> {
    let build = pin(project)?;
    fetch(&Base::from_env(), &keys(), &build, &project.join(DIR), true)
}

/// The build the project pins: a native mod's `[native] horizon`, any other code mod's
/// `[package.metadata.horizon] sdk` in `Cargo.toml`.
pub(crate) fn pin(project: &Path) -> Result<String, String> {
    let read = |name: &str| {
        let path = project.join(name);
        fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
    };
    let manifest = Manifest::parse(&read(MANIFEST)?)?;
    let build = match manifest.native {
        Some(native) => native.horizon,
        None => {
            let cargo: toml::Table = read("Cargo.toml")?
                .parse()
                .map_err(|e| format!("Cargo.toml: {e}"))?;
            cargo
                .get("package")
                .and_then(|p| p.get("metadata"))
                .and_then(|m| m.get("horizon"))
                .and_then(|h| h.get("sdk"))
                .and_then(toml::Value::as_str)
                .ok_or("Cargo.toml has no [package.metadata.horizon] sdk: the SDK build this mod is made for")?
                .to_owned()
        }
    };
    if !horizon_mod_package::manifest::revision(&build) {
        return Err(format!(
            "the pinned SDK build `{build}` is not a full commit sha (40 lowercase hex digits)"
        ));
    }
    Ok(build)
}

/// Pins the project at `project` to `build`, a full sha, and drops the SDK of the old pin: the
/// next command fetches the new one.
pub(crate) fn repin(project: &Path, build: &str) -> Result<(), String> {
    // The pin's own comment and spacing stay: only its value changes.
    let edit = |name: &str, pin: &dyn Fn(&mut toml_edit::DocumentMut) -> &mut toml_edit::Item| {
        let path = project.join(name);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut doc: toml_edit::DocumentMut = text
            .parse()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let item = pin(&mut doc);
        match item.as_value_mut() {
            Some(value) => crate::internals::set_string(value, build),
            None => *item = toml_edit::value(build),
        }
        fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))
    };
    let manifest = Manifest::parse(
        &fs::read_to_string(project.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?,
    )?;
    if manifest.native.is_some() {
        edit(MANIFEST, &|doc| &mut doc["native"]["horizon"])?;
    } else {
        edit("Cargo.toml", &|doc| {
            &mut doc["package"]["metadata"]["horizon"]["sdk"]
        })?;
    }
    match fs::remove_dir_all(project.join(DIR)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{DIR}: {e}")),
        _ => Ok(()),
    }
}

/// The build id `text` names: a full sha as written; `nightly` or `nightly-<date>` through that
/// Nightly's `sdk.json`; a short sha through the releases listed.
pub(crate) fn resolve(base: &Base, text: &str) -> Result<String, String> {
    let hex = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    };
    if horizon_mod_package::manifest::revision(text) {
        return Ok(text.to_owned());
    }
    // A Nightly name becomes a release tag, so a path segment: only its exact forms are taken.
    let nightly = text == "nightly"
        || text.strip_prefix("nightly-").is_some_and(|date| {
            date.len() == 10
                && date.bytes().enumerate().all(|(i, b)| match i {
                    4 | 7 => b == b'-',
                    _ => b.is_ascii_digit(),
                })
        });
    if nightly {
        let bytes = base.read(text, sdk::INDEX, SMALL).map_err(|e| {
            format!(
                "Nightly {text} has no SDK ({e}). Pin a build that has one: --horizon <full sha>"
            )
        })?;
        #[derive(serde::Deserialize)]
        struct Named {
            build: Commit,
        }
        #[derive(serde::Deserialize)]
        struct Commit {
            commit: String,
        }
        // Only a pointer: the build it names is verified when it is fetched.
        let named: Named = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Nightly {text}'s {}: {e}", sdk::INDEX))?;
        if !horizon_mod_package::manifest::revision(&named.build.commit) {
            return Err(format!("Nightly {text}'s {} names no build", sdk::INDEX));
        }
        return Ok(named.build.commit);
    }
    if (7..40).contains(&text.len()) && hex(text) {
        let found: Vec<String> = base
            .releases()?
            .into_iter()
            .filter_map(|tag| tag.strip_prefix("sdk-").map(str::to_owned))
            .filter(|build| {
                build.starts_with(text) && horizon_mod_package::manifest::revision(build)
            })
            .collect();
        return match &found[..] {
            [one] => Ok(one.clone()),
            [] => Err(format!("no SDK release for a build starting {text}")),
            _ => Err(format!(
                "{text} names {} SDK builds; write more of the sha",
                found.len()
            )),
        };
    }
    Err(format!(
        "`{text}` is not a build: write `nightly`, `nightly-<YYYY-MM-DD>`, or a commit sha (7 to 40 hex digits)"
    ))
}

/// The release keys an SDK must be signed with.
pub(crate) fn keys() -> Vec<PublicKey> {
    horizon_release_sign::trusted_keys_for(native::PREBUILT_REPO)
}

/// Where SDK releases are read from.
pub(crate) enum Base {
    /// `<url>/<release>/<file>`.
    Url(String),
    /// `<folder>/<release>/<file>`.
    Folder(PathBuf),
}

impl Base {
    /// `HORIZON_SDK_BASE`, a URL (`http://` or `https://`) or a folder, else GitHub's releases.
    pub(crate) fn from_env() -> Self {
        match std::env::var("HORIZON_SDK_BASE") {
            Ok(base) if base.starts_with("https://") || base.starts_with("http://") => {
                Self::Url(base.trim_end_matches('/').to_owned())
            }
            Ok(base) if !base.is_empty() => Self::Folder(base.into()),
            _ => Self::Url(BASE.to_owned()),
        }
    }

    fn open(&self, release: &str, file: &str) -> Result<Box<dyn Read>, String> {
        match self {
            Self::Url(base) => {
                let url = format!("{base}/{release}/{file}");
                let response = ureq::get(&url).call().map_err(|e| format!("{url}: {e}"))?;
                Ok(Box::new(response.into_reader()))
            }
            Self::Folder(base) => {
                let path = base.join(release).join(file);
                let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Box::new(file))
            }
        }
    }

    /// The release's file, refused past `limit` bytes.
    fn read(&self, release: &str, file: &str, limit: u64) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        self.open(release, file)?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{release}/{file}: {e}"))?;
        if bytes.len() as u64 > limit {
            return Err(format!("{release}/{file} is larger than {limit} bytes"));
        }
        Ok(bytes)
    }

    /// The release names there are: a folder's entries, or GitHub's list for its base.
    fn releases(&self) -> Result<Vec<String>, String> {
        match self {
            Self::Folder(base) => Ok(fs::read_dir(base)
                .map_err(|e| format!("{}: {e}", base.display()))?
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .collect()),
            Self::Url(base) if base == BASE => {
                let api = horizon_addresses::SDK_RELEASE_LIST;
                let text = ureq::get(api)
                    .call()
                    .map_err(|e| format!("{api}: {e}"))?
                    .into_string()
                    .map_err(|e| e.to_string())?;
                let releases: Vec<serde_json::Value> =
                    serde_json::from_str(&text).map_err(|e| format!("{api}: {e}"))?;
                Ok(releases
                    .iter()
                    .filter_map(|release| release["tag_name"].as_str().map(str::to_owned))
                    .collect())
            }
            Self::Url(base) => Err(format!(
                "{base} lists no releases: write the build's full sha"
            )),
        }
    }
}

/// `build`'s verified SDK in `dir`: reused when `dir`'s stamp names it, else fetched afresh. With
/// `api`, the API source is unpacked too; without, only the small files are fetched.
pub(crate) fn fetch(
    base: &Base,
    keys: &[PublicKey],
    build: &str,
    dir: &Path,
    api: bool,
) -> Result<PinnedSdk, String> {
    let tag = format!("sdk-{build}");
    let stamped = fs::read_to_string(dir.join(STAMP)).is_ok_and(|stamp| stamp == build);
    if stamped {
        let bytes = fs::read(dir.join(sdk::INDEX)).map_err(|e| format!("{DIR}: {e}"))?;
        let index = Index::read(&bytes, build).map_err(|e| format!("{tag}: {e}"))?;
        return Ok(pinned(&index, dir));
    }
    if let Err(e) = fs::remove_dir_all(dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(format!("{}: {e}", dir.display()));
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let sums = base.read(&tag, horizon_release_sign::SUMS, SMALL)?;
    let signature = base.read(&tag, horizon_release_sign::SIGNATURE, SMALL)?;
    let listed =
        horizon_release_sign::verify(&tag, &sums, &String::from_utf8_lossy(&signature), keys)?;
    let bytes = base.read(&tag, sdk::INDEX, SMALL)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if !listed
        .iter()
        .any(|(name, sha)| name == sdk::INDEX && *sha == digest)
    {
        return Err(format!(
            "{tag}: its {} is not the one its signed {} list",
            sdk::INDEX,
            horizon_release_sign::SUMS
        ));
    }
    let index = Index::read(&bytes, build).map_err(|e| format!("{tag}: {e}"))?;
    let surface = base.read(&tag, SURFACE, SMALL)?;
    index
        .verify(SURFACE, surface.as_slice())
        .map_err(|e| format!("{tag}: {e}"))?;
    let write = |name: &str, bytes: &[u8]| {
        fs::write(dir.join(name), bytes).map_err(|e| format!("{}: {e}", dir.join(name).display()))
    };
    write(SURFACE, &surface)?;
    if api {
        let archive = dir.join(API_SOURCE);
        // One byte past the listed size is enough for the check to refuse it.
        let size = index
            .asset(API_SOURCE)
            .map_err(|e| format!("{tag}: {e}"))?
            .size;
        let mut body = base.open(&tag, API_SOURCE)?.take(size.saturating_add(1));
        let mut file =
            fs::File::create(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
        std::io::copy(&mut body, &mut file).map_err(|e| format!("{tag}/{API_SOURCE}: {e}"))?;
        drop(file);
        sdk::unpack(&index, API_SOURCE, &archive, &dir.join("api"), API_LIMIT)
            .map_err(|e| format!("{tag}: {e}"))?;
        fs::remove_file(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    }
    write(sdk::INDEX, &bytes)?;
    write(STAMP, build.as_bytes())?;
    Ok(pinned(&index, dir))
}

fn pinned(index: &Index, dir: &Path) -> PinnedSdk {
    let build = index.build.commit.clone();
    let short = &build[..7];
    PinnedSdk {
        name: match &index.build.nightly {
            Some(nightly) => format!("{nightly} ({short})"),
            None => short.to_owned(),
        },
        build,
        dir: dir.to_owned(),
        surface: dir.join(SURFACE),
    }
}

/// `text` (a Nightly name or a sha)'s build, its small files only (`sdk.json`, `surface.json`),
/// in the project's `.horizon/cache/<build>/`, verified as the pinned SDK is, without repinning.
pub(crate) fn fetch_build(project: &Path, text: &str) -> Result<PinnedSdk, String> {
    let base = Base::from_env();
    let build = resolve(&base, text)?;
    fetch(
        &base,
        &keys(),
        &build,
        &project.join(".horizon/cache").join(&build),
        false,
    )
}

#[cfg(test)]
mod tests;
