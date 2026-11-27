use std::{collections::HashMap, sync::Arc};
use sunrise_client::skins;
use sunrise_core::profile::AccountProperty;
use sunrise_gameplay_common::game::remote::object_skin_changed;
use sunrise_server_files::profiles::profile_key_keys;
use {
    sunrise_client_gameplay::game::client_side::remote_client::session_state::PlayerTextures,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::ServicesKeys,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::skin_entries,
};

// Offline test data: two independently generated RSA keys and SHA1withRSA
// signatures, on the base64 property string (not its decoded JSON).
const SIGNED_TEXTURES: &str = include_str!("fixtures/signed_textures.json");

/// The skin downloader, its cache root never reached: these tests download nothing.
fn downloads() -> skins::Downloader {
    skins::Downloader::new(tempfile::tempdir().unwrap().path().to_path_buf())
}

/// Asks a property until the thread that unpacks it answers.
fn settled(
    downloads: &mut skins::Downloader,
    property: &PlayerTextures,
    keys: &ServicesKeys,
    require_secure: bool,
) -> Arc<skins::Textures> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(textures) = property.unpack(downloads, keys, require_secure) {
            return textures;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the property's textures never arrived"
        );
        std::thread::yield_now();
    }
}

/// An NPC plugin lists the NPC, spawns it, and takes the entry off the list:
/// the entity keeps the skin it found; a player spawned before its entry
/// arrives takes the entry once it comes.
#[test]
fn a_player_entity_keeps_the_skin_its_entry_gave() {
    let property = |value: &str| {
        Arc::new(PlayerTextures::new(
            7,
            AccountProperty {
                name: "textures".into(),
                value: value.into(),
                signature: None,
            },
        ))
    };
    let npc = property("npc");
    let (mut listed, mut kept) = (HashMap::from([(7u128, npc.clone())]), HashMap::new());
    let mut players = HashMap::from([(2i32, 7u128), (3, 8)]);
    let assert_npc = |entries: Vec<(u128, Arc<PlayerTextures>)>| {
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, 7);
        assert!(Arc::ptr_eq(&entries[0].1, &npc));
    };
    assert_npc(skin_entries(&players, &listed, &mut kept));
    listed.remove(&7);
    assert_npc(skin_entries(&players, &listed, &mut kept));
    // Player 8's entry comes after its entity.
    let late = property("late");
    listed.insert(8, late.clone());
    let mut entries = skin_entries(&players, &listed, &mut kept);
    entries.sort_by_key(|(u, _)| *u);
    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].0, entries[1].0), (7, 8));
    assert!(Arc::ptr_eq(&entries[0].1, &npc));
    assert!(Arc::ptr_eq(&entries[1].1, &late));
    // A removed entity forgets its profile with it.
    players.remove(&2);
    kept.remove(&2);
    listed.clear();
    assert_eq!(skin_entries(&players, &listed, &mut kept).len(), 1);
}

#[test]
fn texture_signatures_use_only_the_profile_property_keys() {
    use skins::Signature;
    let fixture: serde_json::Value = serde_json::from_str(SIGNED_TEXTURES).unwrap();
    let keys = ServicesKeys::Ready(profile_key_keys(&fixture["public_keys"].to_string()).unwrap());
    let value = fixture["value"].as_str().unwrap();
    let good = fixture["profile_property_signature"].as_str().unwrap();
    let cert = fixture["player_certificate_signature"].as_str().unwrap();
    let altered = fixture["altered_value"].as_str().unwrap();

    let mut downloads = downloads();
    let classify = |downloads: &mut skins::Downloader,
                    value: &str,
                    signature: Option<&str>,
                    keys: &ServicesKeys| {
        let property = PlayerTextures::new(
            7,
            AccountProperty {
                name: "textures".into(),
                value: value.into(),
                signature: signature.map(str::to_owned),
            },
        );
        settled(downloads, &property, keys, true).signature
    };
    assert_eq!(
        classify(&mut downloads, value, None, &keys),
        Signature::Unsigned
    );
    assert_eq!(
        classify(&mut downloads, value, Some(good), &keys),
        Signature::Signed
    );
    assert_eq!(
        classify(&mut downloads, value, Some(cert), &keys),
        Signature::Invalid
    );
    assert_eq!(
        classify(&mut downloads, altered, Some(good), &keys),
        Signature::Invalid
    );
    // A new key set replaces cached verdicts (as `fresh_services_keys` does per connection).
    downloads.forget_unpacked();
    assert_eq!(
        classify(
            &mut downloads,
            value,
            Some(good),
            &ServicesKeys::Ready(Default::default())
        ),
        Signature::Invalid
    );
    assert_eq!(
        classify(&mut downloads, value, Some("not base64"), &keys),
        Signature::Invalid
    );
    // The decoded payload is unchanged, but the signature is over the wire string.
    assert_eq!(
        classify(
            &mut downloads,
            value.trim_end_matches('='),
            Some(good),
            &keys
        ),
        Signature::Invalid
    );
}

#[test]
fn signed_texture_retained_by_entity_resolves_when_keys_arrive() {
    use skins::{Download, SkinLookup, lookup};
    let fixture: serde_json::Value = serde_json::from_str(SIGNED_TEXTURES).unwrap();
    let property = Arc::new(PlayerTextures::new(
        7,
        AccountProperty {
            name: "textures".into(),
            value: fixture["value"].as_str().unwrap().into(),
            signature: Some(
                fixture["profile_property_signature"]
                    .as_str()
                    .unwrap()
                    .into(),
            ),
        },
    ));
    let mut downloads = downloads();
    let players = HashMap::from([(2, 7)]);
    let mut kept = HashMap::new();
    let mut listed = HashMap::from([(7, property)]);
    let entries = skin_entries(&players, &listed, &mut kept);
    let retained = entries[0].1.clone();
    assert!(
        retained
            .unpack(&mut downloads, &ServicesKeys::Pending, true)
            .is_none(),
        "a signature waits for the keys: no verdict is cached"
    );
    listed.clear();

    let keys = ServicesKeys::Ready(profile_key_keys(&fixture["public_keys"].to_string()).unwrap());
    let entries = skin_entries(&players, &listed, &mut kept);
    assert!(
        Arc::ptr_eq(&retained, &entries[0].1),
        "the entity kept the same property"
    );
    let textures = settled(&mut downloads, &entries[0].1, &keys, true);
    let SkinLookup::Ready(resolved) = lookup(&textures, true, |_, _| Download::Ready) else {
        panic!("the retained signed property must pass the secure lookup once keys load");
    };
    assert_eq!(resolved.cape, Some(skins::cape_location("abcd1234")));
}

#[test]
fn tab_publication_fingerprint_tracks_all_inputs() {
    let fingerprint = |ids: &[u128], skins, atlas| {
        (
            ids.len(),
            ids.iter().fold(0, |xor, uuid| xor ^ uuid),
            skins,
            atlas,
        )
    };
    let published = fingerprint(&[7, 8], 1, 2);
    assert_eq!(published, fingerprint(&[8, 7], 1, 2));
    assert_ne!(published, fingerprint(&[7, 9], 1, 2));
    assert_ne!(published, fingerprint(&[7, 8], 2, 2));
    assert_ne!(published, fingerprint(&[7, 8], 1, 3));
}

#[test]
fn tab_object_skin_publication_tracks_changes_and_atlas_clear() {
    let mut published = std::collections::HashMap::new();
    let mut writes = 0;
    for skin in ["skins/first", "skins/first", "skins/second"] {
        for key in ["id:7", "name:player"] {
            if object_skin_changed(&published, key, skin) {
                published.insert(key.to_owned(), skin.to_owned());
                writes += 1;
            }
        }
    }
    assert_eq!(writes, 4, "unchanged skins do not publish either entry");
    published.clear();
    assert!(object_skin_changed(&published, "id:7", "skins/second"));
    assert!(object_skin_changed(
        &published,
        "name:player",
        "skins/second"
    ));
}

/// The services' keys come the way a joined client's do: nothing is cached on keys
/// it does not have, a signed property resolves once they arrive, and the local
/// player's lookup does not wait for them.
#[test]
fn a_client_waits_for_the_services_keys_except_for_the_local_player() {
    use skins::{Download, Signature, SkinLookup, lookup};
    use sunrise_server_files::profiles::ServicesKeySet;
    let fixture: serde_json::Value = serde_json::from_str(SIGNED_TEXTURES).unwrap();
    let (answer, asked) = std::sync::mpsc::channel::<ServicesKeySet>();
    let signed = |profile: u128| {
        PlayerTextures::new(
            profile,
            AccountProperty {
                name: "textures".into(),
                value: fixture["value"].as_str().unwrap().into(),
                signature: Some(
                    fixture["profile_property_signature"]
                        .as_str()
                        .unwrap()
                        .into(),
                ),
            },
        )
    };
    let mut downloads = downloads();
    let property = signed(7);
    let keys = ServicesKeys::fetching_with(move || Ok::<_, &'static str>(asked.recv().unwrap()));
    assert!(matches!(*keys.lock().unwrap(), ServicesKeys::Pending));
    assert!(
        property
            .unpack(&mut downloads, &keys.lock().unwrap(), true)
            .is_none(),
        "empty ready keys would cache the signature as invalid"
    );
    // The local player's own profile (1) is asked again every frame: cached too.
    assert_eq!(
        settled(&mut downloads, &signed(1), &keys.lock().unwrap(), false).signature,
        Signature::Invalid,
        "PlayerInfo.createSkinLookup: requireSecure is false for the local player"
    );
    answer
        .send(profile_key_keys(&fixture["public_keys"].to_string()).unwrap())
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while matches!(*keys.lock().unwrap(), ServicesKeys::Pending) {
        assert!(
            std::time::Instant::now() < deadline,
            "the keys never arrived"
        );
        std::thread::yield_now();
    }
    let textures = settled(&mut downloads, &property, &keys.lock().unwrap(), true);
    let SkinLookup::Ready(resolved) = lookup(&textures, true, |_, _| Download::Ready) else {
        panic!("the signed property must resolve once the keys arrive");
    };
    assert_eq!(resolved.cape, Some(skins::cape_location("abcd1234")));
}

/// `SkinManager.getOrLoad`: the game thread that asks for a signed property never verifies it.
/// It reads nothing at once, then the verdict of the thread that unpacked the property: the
/// signed one passes the secure lookup, a tampered one is refused; a profile taken off the list
/// and listed again, or the same property under another of its holders, is answered at once.
#[test]
fn a_signed_property_is_verified_off_the_game_thread() {
    use skins::{Download, Signature, SkinLookup, lookup};
    let fixture: serde_json::Value = serde_json::from_str(SIGNED_TEXTURES).unwrap();
    let keys = ServicesKeys::Ready(profile_key_keys(&fixture["public_keys"].to_string()).unwrap());
    let signed = |value: &str| {
        PlayerTextures::new(
            7,
            AccountProperty {
                name: "textures".into(),
                value: value.into(),
                signature: Some(
                    fixture["profile_property_signature"]
                        .as_str()
                        .unwrap()
                        .into(),
                ),
            },
        )
    };
    let (good, tampered) = (
        signed(fixture["value"].as_str().unwrap()),
        signed(fixture["altered_value"].as_str().unwrap()),
    );
    let mut downloads = downloads();
    // The frame the list arrives in: both are asked, and neither is answered.
    assert!(
        good.unpack(&mut downloads, &keys, true).is_none()
            && tampered.unpack(&mut downloads, &keys, true).is_none(),
        "no signature is verified on the asking thread"
    );
    let textures = settled(&mut downloads, &good, &keys, true);
    assert_eq!(textures.signature, Signature::Signed);
    assert!(matches!(
        lookup(&textures, true, |_, _| Download::Ready),
        SkinLookup::Ready(_)
    ));
    let textures = settled(&mut downloads, &tampered, &keys, true);
    assert_eq!(textures.signature, Signature::Invalid);
    assert_eq!(
        lookup(&textures, true, |_, _| Download::Ready),
        SkinLookup::Disallowed
    );
    // PlayerInfoRemove, then PlayerInfoUpdate with the same property: the cache's answer.
    let relisted = signed(fixture["value"].as_str().unwrap());
    assert_eq!(
        relisted
            .unpack(&mut downloads, &keys, true)
            .unwrap()
            .signature,
        Signature::Signed
    );
}
