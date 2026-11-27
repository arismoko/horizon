use super::*;
use std::{cell::RefCell, rc::Rc};

/// The first index the registry published, signed with its production key.
const FIRST: &[u8] = include_bytes!("first-index.json");
const FIRST_SIGNATURE: &[u8] = include_bytes!("first-index.json.minisig");
const FIRST_SERIAL: u64 = 1795408124;
const URL: &str = "https://example.org/v2/index.signed";

/// `index.signed` as the registry publishes it: the signature file, then the index's bytes.
fn serve(index: &'static [u8]) -> impl Fn(&str, u64) -> Result<Vec<u8>, String> {
    move |url, limit| {
        assert_eq!(url, URL);
        let signed = [FIRST_SIGNATURE, index].concat();
        assert!(signed.len() as u64 <= limit);
        Ok(signed)
    }
}

pub(crate) fn read_first_index(cache: &Path) -> Result<Index, String> {
    read_index(cache, URL, &serve(FIRST), KEY)
}

#[test]
fn the_index_needs_the_registry_signature_and_a_serial_no_older_than_the_last_accepted() {
    let cache = tempfile::tempdir().unwrap();
    let serial = cache.path().join(SERIAL);
    let index = read_first_index(cache.path()).unwrap();
    assert_eq!(index.serial, FIRST_SERIAL);
    assert_eq!(
        fs::read_to_string(&serial).unwrap(),
        FIRST_SERIAL.to_string()
    );

    // Every byte is signed, whitespace included: the bytes are never reserialized to check them.
    let edited: &'static [u8] = Vec::leak(FIRST.iter().copied().chain(*b" ").collect());
    let error = read_index(cache.path(), URL, &serve(edited), KEY).unwrap_err();
    assert!(error.contains("registry's key"), "{error}");

    fs::write(&serial, (FIRST_SERIAL + 1).to_string()).unwrap();
    let error = read_first_index(cache.path()).unwrap_err();
    assert!(
        error.contains(&format!(
            "serial {FIRST_SERIAL} is older than {}",
            FIRST_SERIAL + 1
        )),
        "{error}"
    );
    fs::write(&serial, (FIRST_SERIAL - 1).to_string()).unwrap();
    read_first_index(cache.path()).unwrap();
    assert_eq!(
        fs::read_to_string(&serial).unwrap(),
        FIRST_SERIAL.to_string()
    );
    // An unreadable floor is named, never taken as no floor.
    fs::write(&serial, "").unwrap();
    let error = read_first_index(cache.path()).unwrap_err();
    assert!(error.contains(&serial.display().to_string()), "{error}");
}

fn library(bytes: &[u8]) -> Index {
    serde_json::from_value(serde_json::json!({
        "schema": 2, "serial": 1, "revoked": [],
        "projects": [{"id": "Ab3dE9xQ", "mod": "library", "kind": "native-mod",
            "repository": {"id": 101, "name": "author/library"},
            "releases": [{"version": "1.2.0",
                "workflow": "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v1",
                "file": {"url": "https://example.org/library.smod", "size": bytes.len(),
                    "sha256": index::sha256(bytes), "attestation": "https://example.org/library.smod.sigstore.json"}}]}]
    }))
    .unwrap()
}

#[test]
fn a_package_is_downloaded_once_and_its_size_digest_and_attestation_are_checked_on_every_use() {
    let bytes = b"package bytes".to_vec();
    let fetched = Rc::new(RefCell::new(Vec::new()));
    let attested = Rc::new(RefCell::new(Vec::new()));
    let (served, seen) = (bytes.clone(), fetched.clone());
    let checked = attested.clone();
    let registry = Registry::fixture(
        library(&bytes),
        move |url, limit| {
            seen.borrow_mut().push((url.to_owned(), limit));
            Ok(match url.ends_with(".sigstore.json") {
                true => b"bundle".to_vec(),
                false => served.clone(),
            })
        },
        move |bundle, digest, project, release| {
            assert_eq!(bundle, b"bundle");
            checked
                .borrow_mut()
                .push((digest, project.repository.id, release.version.to_string()));
            Ok(())
        },
    );
    let cache = tempfile::tempdir().unwrap();
    let project = &registry.index.projects[0];
    let release = &project.releases[0];
    let archive = registry.package(cache.path(), project, release).unwrap();
    let kept = registry.package(cache.path(), project, release).unwrap();
    assert_eq!(
        (fs::read(&archive).unwrap(), &kept),
        (bytes.clone(), &archive)
    );
    assert_eq!(
        *fetched.borrow(),
        [
            (
                "https://example.org/library.smod".to_owned(),
                bytes.len() as u64
            ),
            (
                "https://example.org/library.smod.sigstore.json".to_owned(),
                index::MAX_FILE
            ),
        ]
    );
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(
        *attested.borrow(),
        [(digest, 101, "1.2.0".into()), (digest, 101, "1.2.0".into())]
    );

    let mut edited = bytes.clone();
    edited[0] ^= 1;
    fs::write(&archive, edited).unwrap();
    let error = registry
        .package(cache.path(), project, release)
        .unwrap_err();
    assert!(
        error.contains("library 1.2.0 (registry project Ab3dE9xQ)") && error.contains("SHA-256"),
        "{error}"
    );

    let refused = Registry::fixture(
        library(&bytes),
        move |url, _| {
            Ok(match url.ends_with(".sigstore.json") {
                true => b"bundle".to_vec(),
                false => bytes.clone(),
            })
        },
        |_, _, _, _| Err("attestation: names another repository".into()),
    );
    let error = refused
        .package(tempfile::tempdir().unwrap().path(), project, release)
        .unwrap_err();
    assert!(error.contains("names another repository"), "{error}");
}

/// The production path end to end: the live index under the built-in key, then one live
/// release's package and its GitHub attestation. The path is the same for every kind of project,
/// so the release is whichever the registry lists first.
#[test]
#[ignore = "daily"]
fn a_live_release_verifies_against_the_live_index_and_its_attestation() {
    let cache = tempfile::tempdir().unwrap();
    let registry = Registry::read(cache.path()).unwrap();
    let (project, release) = registry
        .index
        .projects
        .iter()
        .find_map(|p| Some((p, p.releases.first()?)))
        .expect("a release in the registry");
    let archive = registry.package(cache.path(), project, release).unwrap();
    assert_eq!(
        sunrise_mod_package::Package::read(&archive)
            .unwrap()
            .manifest
            .id,
        project.mod_id
    );
}
