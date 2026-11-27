//! A world Horizon saved keeps its mods' attachments (`horizon_names`).
use super::*;

/// A mod's attachments as Horizon or Sunrise saved them: `quests` holds `progress`.
fn saved(progress: i32) -> Tag {
    Tag::compound([("quests", Tag::compound([("progress", Tag::Int(progress))]))])
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sunrise-horizon-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_horizon_chunk_and_its_block_entities_keep_their_attachments() {
    let mut record = NamedTag {
        name: JavaString::default(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("horizon:attachments", saved(1)),
            (
                "block_entities",
                Tag::List(vec![
                    Tag::compound([
                        ("id", string("minecraft:chest")),
                        ("horizon:attachments", saved(2)),
                    ]),
                    // Saved under both names: Sunrise's is the newer.
                    Tag::compound([
                        ("id", string("minecraft:barrel")),
                        ("horizon:attachments", saved(3)),
                        (ATTACHMENTS_KEY, saved(4)),
                    ]),
                ]),
            ),
        ]),
    };
    upgrade_record(Folder::Region, &mut record, || None).unwrap();
    assert_eq!(record.tag.get(ATTACHMENTS_KEY), Some(&saved(1)));
    assert_eq!(record.tag.get("horizon:attachments"), None);
    let Some(Tag::List(block_entities)) = record.tag.get("block_entities") else {
        panic!("no block entities");
    };
    assert_eq!(block_entities[0].get(ATTACHMENTS_KEY), Some(&saved(2)));
    assert_eq!(block_entities[1].get(ATTACHMENTS_KEY), Some(&saved(4)));
    assert_eq!(block_entities[1].get("horizon:attachments"), None);
}

#[test]
fn clearing_the_sunrise_key_does_not_restore_the_horizon_value() {
    let mut player = Tag::compound([
        ("horizon:attachments", saved(10)),
        (ATTACHMENTS_KEY, saved(11)),
    ]);
    horizon_names::adopt_in_player(&mut player);
    assert_eq!(player.get(ATTACHMENTS_KEY), Some(&saved(11)));

    let Tag::Compound(data) = &mut player else {
        panic!("expected a player compound");
    };
    data.remove(&ATTACHMENTS_KEY.into());
    horizon_names::adopt_in_player(&mut player);
    assert_eq!(player.get(ATTACHMENTS_KEY), None);
    assert_eq!(player.get("horizon:attachments"), None);
}

#[test]
fn horizon_entities_and_their_passengers_keep_their_attachments() {
    let rider = Tag::compound([
        ("id", string("minecraft:zombie")),
        ("horizon:attachments", saved(6)),
    ]);
    let horse = Tag::compound([
        ("id", string("minecraft:horse")),
        ("horizon:attachments", saved(5)),
        ("Passengers", Tag::List(vec![rider])),
    ]);
    let mut record = NamedTag {
        name: JavaString::default(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("Entities", Tag::List(vec![horse])),
        ]),
    };
    upgrade_record(Folder::Entities, &mut record, || None).unwrap();
    let Some(Tag::List(entities)) = record.tag.get("Entities") else {
        panic!("no entities");
    };
    assert_eq!(entities[0].get(ATTACHMENTS_KEY), Some(&saved(5)));
    let Some(Tag::List(passengers)) = entities[0].get("Passengers") else {
        panic!("no passengers");
    };
    assert_eq!(passengers[0].get(ATTACHMENTS_KEY), Some(&saved(6)));
}

#[test]
fn a_horizon_player_and_their_mount_keep_their_attachments() {
    let dir = scratch("player");
    let player = Tag::compound([
        ("horizon:attachments", saved(7)),
        (
            "RootVehicle",
            Tag::compound([(
                "Entity",
                Tag::compound([
                    ("id", string("minecraft:pig")),
                    ("horizon:attachments", saved(8)),
                ]),
            )]),
        ),
    ]);
    Level {
        name: "Horizon".into(),
        player_extra: Some(player),
        ..Level::default()
    }
    .write(&dir, &Default::default())
    .unwrap();
    let back = Level::read(&dir).unwrap().player_extra.unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(back.get(ATTACHMENTS_KEY), Some(&saved(7)));
    assert_eq!(back.get("horizon:attachments"), None);
    let mount = back
        .get("RootVehicle")
        .and_then(|v| v.get("Entity"))
        .unwrap();
    assert_eq!(mount.get(ATTACHMENTS_KEY), Some(&saved(8)));
}

#[test]
fn a_horizon_dimension_keeps_its_attachments_and_saves_them_under_sunrise() {
    let dir = scratch("level");
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, None);
    let horizon = dir.join("data").join("horizon").join("attachments.dat");
    fs::create_dir_all(horizon.parent().unwrap()).unwrap();
    write_dat(
        &horizon,
        Tag::compound([("data", saved(9)), ("DataVersion", Tag::Int(DATA_VERSION))]),
    )
    .unwrap();
    let mut attachments = save.load_attachments();
    assert_eq!(Tag::Compound(attachments.try_to_nbt().unwrap()), saved(9));
    // Emptied by its mods, it stays empty: the Horizon file is not read again.
    attachments = Default::default();
    save.save_attachments(&attachments).unwrap();
    let reopened = save.load_attachments();
    let _ = fs::remove_dir_all(&dir);
    assert!(reopened.try_to_nbt().unwrap().is_empty());
}
