// `lan.rs`'s tests (included: that file may not grow).

/// `SulfurCube`'s entity data after `AbstractCubeMob.ID_SIZE` (18), in the order
/// the class defines it: `MAX_FUSE` (19, an int, -1 until primed), then
/// `FROM_BUCKET` (20, a boolean). A 26.3 client reads each index with its own
/// serializer, and a boolean at 19 disconnected one (#13721).
#[test]
fn a_sulfur_cube_sends_its_fuse_at_19_and_from_bucket_at_20() {
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::entity_data::{Value, serializer};
    use sunrise_sim::mobs::Mob;
    let root = std::env::temp_dir().join(format!("sunrise-sulfur-meta-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("entity-types.json"),
        r#"{"minecraft:sulfur_cube":{"width":0.52,"height":0.52,"eye_height":0.325,"category":"monster","fire_immune":false,"attributes":{"minecraft:max_health":4.0}}}"#,
    )
    .unwrap();
    // No attribute catalog: the rows' values stay as written.
    let attributes = &std::sync::Arc::new(
        sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [],
            "entities": {},
        }))
        .unwrap(),
    );
    let types = sunrise_sim::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let kind = std::sync::Arc::new(types.types["minecraft:sulfur_cube"].clone());
    let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut cube = Mob::new(1, kind, at, 0.0);
    let hub = super::tests::meta_hub("SulfurCubeMetaTest");
    let meta_of = |m: &Mob| {
        let (mut meta, mut defaults) = (Vec::new(), Vec::new());
        sunrise_gameplay_common::game::lan_mob_metadata::mob_meta(
            m,
            &hub,
            None,
            0,
            &mut meta,
            &mut defaults,
        );
        (meta, defaults)
    };

    // A fresh cube: both entries are at their defaults.
    let (meta, defaults) = meta_of(&cube);
    assert!(
        defaults.contains(&(19, serializer::INT, Value::Int(-1))),
        "{defaults:?}"
    );
    assert!(
        defaults.contains(&(20, serializer::BOOLEAN, Value::Bool(false))),
        "{defaults:?}"
    );
    assert!(meta.iter().all(|e| e.0 != 19 && e.0 != 20), "{meta:?}");

    // A saved cube, out of a bucket and primed with 40 ticks to go.
    let saved = Tag::compound([("from_bucket", Tag::Byte(1)), ("fuse", Tag::Int(40))]);
    sunrise_sim::mobs::sulfur_cube::from_nbt(&mut cube, &saved);
    let (meta, _) = meta_of(&cube);
    assert!(
        meta.contains(&(19, serializer::INT, Value::Int(40))),
        "{meta:?}"
    );
    assert!(
        meta.contains(&(20, serializer::BOOLEAN, Value::Bool(true))),
        "{meta:?}"
    );
}

/// Vanilla 26.3 server witness (`VanillaSulfurCubeFuseOracle`, `tools/test_sulfur_cube_fuse_parity.py`).
#[test]
#[ignore = "run by tools/test_sulfur_cube_fuse_parity.py"]
fn a_loaded_sulfur_cube_fuse_matches_the_pinned_oracle() {
    run_a_loaded_sulfur_cube_fuse_matches_the_pinned_oracle();
}

pub(in crate::game::server_side) fn run_a_loaded_sulfur_cube_fuse_matches_the_pinned_oracle() {
    use sunrise_core::nbt::{Tag, parse_snbt};
    use sunrise_protocol::entity_data::Value;
    use sunrise_sim::mobs::{Mob, sulfur_cube};
    let dir = std::env::var_os("SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR").unwrap_or_else(|| {
        panic!(
            "SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR is unset; run tools/test_sulfur_cube_fuse_parity.py"
        )
    });
    let path = std::path::PathBuf::from(dir).join("sulfur_cube_fuse_server.txt");
    let witness = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "failed to read witness {} (run tools/test_sulfur_cube_fuse_parity.py): {e}",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    let root = std::env::temp_dir().join(format!("sunrise-sulfur-fuse-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("entity-types.json"),
        r#"{"minecraft:sulfur_cube":{"width":0.52,"height":0.52,"eye_height":0.325,"category":"monster","fire_immune":false,"attributes":{"minecraft:max_health":4.0}}}"#,
    )
    .unwrap();
    let attributes = &std::sync::Arc::new(
        sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [],
            "entities": {},
        }))
        .unwrap(),
    );
    let types = sunrise_sim::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let types: std::collections::HashMap<String, std::sync::Arc<sunrise_sim::mobs::MobType>> =
        types
            .types
            .iter()
            .map(|(id, kind)| (id.clone(), std::sync::Arc::new(kind.clone())))
            .collect();
    let hub = super::tests::meta_hub("SulfurCubeFuseTest");
    let meta_of = |m: &Mob| {
        let (mut meta, mut defaults) = (Vec::new(), Vec::new());
        sunrise_gameplay_common::game::lan_mob_metadata::mob_meta(
            m,
            &hub,
            None,
            0,
            &mut meta,
            &mut defaults,
        );
        (meta, defaults)
    };
    let max_fuse = |meta: &[(u8, i32, Value)]| match meta.iter().find(|e| e.0 == 19) {
        Some((_, _, Value::Int(v))) => v.to_string(),
        _ => "-".into(),
    };
    // `send_dirty`: a changed or new entry, or the default of one that dropped out.
    let sent =
        |before: &[(u8, i32, Value)], after: &[(u8, i32, Value)], defaults: &[(u8, i32, Value)]| {
            if let Some(entry) = after.iter().find(|e| e.0 == 19) {
                return if before.contains(entry) {
                    "-".to_owned()
                } else {
                    max_fuse(after)
                };
            }
            if before.iter().any(|e| e.0 == 19) {
                return max_fuse(defaults);
            }
            "-".into()
        };
    let load = |fuse: Option<&str>, invulnerable: Option<&str>| {
        let mut snbt = String::from(r#"{id:"minecraft:sulfur_cube",Pos:[0.5d,64d,0.5d],NoAI:1b"#);
        if let Some(fuse) = fuse {
            snbt += &format!(",fuse:{fuse}");
        }
        if let Some(invulnerable) = invulnerable {
            snbt += &format!(",Invulnerable:{invulnerable}");
        }
        let tag = parse_snbt(&(snbt + "}")).unwrap().0;
        sunrise_sim::entity_nbt::mob_from_nbt(&tag, &types, 1, None).unwrap()
    };
    let mut rows = vec![format!("# minecraft {}", sunrise_core::VANILLA_VERSION)];
    for line in witness.lines().filter(|line| !line.starts_with('#')) {
        let mut fields = line.split_whitespace();
        let fuse = match fields.next().unwrap() {
            "absent" => None,
            value => Some(value),
        };
        let invulnerable = match fields.next().unwrap() {
            "absent" => None,
            value => Some(value),
        };
        let mut cube = load(fuse, invulnerable);
        let loaded = (
            sulfur_cube::fuse(&cube),
            sulfur_cube::max_fuse(&cube),
            cube.invulnerable_flag,
            sulfur_cube::primed(&cube),
        );
        let tag = sunrise_sim::entity_nbt::mob_to_nbt(
            &cube,
            [1, 2, 3, 4],
            None,
            None,
            |_| None,
            &Default::default(),
        );
        let saved_fuse = match tag.get("fuse") {
            Some(Tag::Int(v)) => v.to_string(),
            _ => "-".into(),
        };
        let saved_invulnerable = match tag.get("Invulnerable") {
            Some(Tag::Byte(v)) => v.to_string(),
            _ => "-".into(),
        };
        let (first, _) = meta_of(&cube);
        for _ in 0..3 {
            sulfur_cube::tick_fuse(&mut cube, &mut Vec::new());
        }
        let (ticked, defaults) = meta_of(&cube);
        let fuse_after = sulfur_cube::fuse(&cube);
        let back = load(Some("-1"), None);
        let (reset, _) = meta_of(&back);
        rows.push(format!(
            "{} {} loaded {} {} {} {} saved {saved_fuse} {saved_invulnerable} late {} ticked {fuse_after} {} back {}",
            fuse.unwrap_or("absent"),
            invulnerable.unwrap_or("absent"),
            loaded.0,
            loaded.1,
            loaded.2,
            loaded.3,
            max_fuse(&first),
            sent(&first, &ticked, &defaults),
            sent(&ticked, &reset, &defaults),
        ));
    }
    let actual = rows.join("\n") + "\n";
    if let Some(path) = std::env::var_os("SUNRISE_SULFUR_CUBE_FUSE_OUTPUT") {
        std::fs::write(path, &actual).unwrap();
    }
    assert!(rows.len() > 1, "empty witness");
    assert_eq!(actual, witness);
}
