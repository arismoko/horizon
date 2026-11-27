use super::*;
use mod_api::{
    AttachmentDescriptor, AttachmentHolder, AttachmentKey, AttachmentQueries, AttachmentQueriesMut,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_mod_runtime::attachments::Holder;
use sunrise_mod_runtime::attachments::{
    ATTACHMENTS_KEY, AttachmentCodec, AttachmentError, AttachmentType,
};

static PROJECTILE_VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "saved_projectile",
    name: "value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |tag| match tag.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync_codec: None,
};

#[test]
#[ignore = "daily"]
fn saved_spit_and_bullet_query_before_access_and_round_trip() {
    with_attachment_game(
        "saved_projectile",
        projectile_descriptor,
        |game, projectile_key| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(Dim::Overworld);
            let contents = Tag::compound([(
                "saved_projectile",
                Tag::compound([(
                    "value",
                    Tag::compound([("value", Tag::compound([("n", Tag::Int(37))]))]),
                )]),
            )]);
            for (n, kind) in ["llama_spit", "shulker_bullet"].into_iter().enumerate() {
                let uuid = [71, 72, 73, n as i32];
                let tag = Tag::compound([
                    (
                        "id",
                        Tag::String(format!("minecraft:{kind}").as_str().into()),
                    ),
                    ("UUID", Tag::IntArray(uuid.to_vec())),
                    (
                        "Pos",
                        Tag::List(
                            [1.0f64, 70.0, 3.0]
                                .map(|n| Tag::Double(n.to_bits()))
                                .to_vec(),
                        ),
                    ),
                    (ATTACHMENTS_KEY, contents.clone()),
                ]);
                game.world.loaded_entities.push(((0, 0), vec![tag.into()]));
                game.spawn_loaded_entities();
                let index = game.attachment_indexes[&Dim::Overworld].clone();
                let key = ("saved_projectile".into(), "value".into());
                let batch = index.lock().unwrap().snapshot(&key);
                assert_eq!(batch.len(), 1, "{kind} before attachment access");
                assert_eq!(batch[0].entry.read(&PROJECTILE_VALUE), Ok(&37), "{kind}");
                let AttachmentHolder::Entity(entity) = batch[0].holder else {
                    panic!("not entity");
                };
                // Change the live map so an opaque raw-NBT echo cannot satisfy the save check.
                projectile_key
                    .set(game, Holder::Entity(entity.id()), 59)
                    .unwrap();
                let saved = game.entity_chunks(&HashSet::from([(0, 0)]), false);
                let saved = saved
                    .iter()
                    .flat_map(|(_, tags)| tags)
                    .find(|tag| sunrise_sim::entity_nbt::uuid(tag) == Some(uuid))
                    .unwrap_or_else(|| panic!("{kind} saved without its UUID"))
                    .clone();
                assert!(game.entity_discard(entity));
                game.native_removals_flush();
                assert!(index.lock().unwrap().snapshot(&key).is_empty());
                // Reload from the serialized data, not a retained in-memory map.
                game.entity_attachments.remove(&entity.id());
                game.world
                    .loaded_entities
                    .push(((0, 0), vec![saved.into()]));
                game.spawn_loaded_entities();
                let restored = index.lock().unwrap().snapshot(&key);
                assert_eq!(restored.len(), 1, "{kind} after reload");
                assert_eq!(
                    restored[0].entry.read(&PROJECTILE_VALUE),
                    Ok(&59),
                    "{kind} saved value"
                );
                let AttachmentHolder::Entity(entity) = restored[0].holder else {
                    panic!("not entity");
                };
                assert!(game.entity_discard(entity));
                game.native_removals_flush();
            }
        },
    );
}

fn projectile_descriptor() -> AttachmentDescriptor<i32> {
    AttachmentDescriptor::new("saved_projectile:value", PROJECTILE_VALUE.default)
        .serialize(PROJECTILE_VALUE.codec.unwrap())
}

fn saved_entity_descriptor() -> AttachmentDescriptor<i32> {
    AttachmentDescriptor::new("saved_entity:value", || 0)
}

struct SavedEntityMod {
    descriptor: fn() -> AttachmentDescriptor<i32>,
    key: Arc<Mutex<Option<AttachmentKey<i32>>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for SavedEntityMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = r.register_attachment_type((self.descriptor)()).unwrap();
        *self.key.lock().unwrap() = Some(key);
    }
}

fn with_attachment_game(
    namespace: &'static str,
    descriptor: fn() -> AttachmentDescriptor<i32>,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &AttachmentKey<i32>),
) {
    let key_slot = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        namespace,
        Box::new(SavedEntityMod {
            descriptor,
            key: key_slot.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let key = key_slot
                .lock()
                .unwrap()
                .clone()
                .expect("attachment registered");
            test(game, &key);
        });
    });
}

#[test]
#[ignore = "daily"]
fn saved_entity_loaders_publish_and_retire_the_authoritative_owner() {
    with_attachment_game(
        "saved_entity",
        saved_entity_descriptor,
        |game, value_key| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(Dim::Overworld);
            for (n, kind) in [
                "item",
                "experience_orb",
                "armor_stand",
                "item_frame",
                "oak_boat",
                "minecart",
                "arrow",
                "small_fireball",
                "snowball",
            ]
            .into_iter()
            .enumerate()
            {
                let uuid = [31, 32, 33, n as i32];
                let tag = Tag::compound([
                    (
                        "id",
                        Tag::String(format!("minecraft:{kind}").as_str().into()),
                    ),
                    ("UUID", Tag::IntArray(uuid.to_vec())),
                    (
                        "Pos",
                        Tag::List(
                            [1.0f64, 70.0, 3.0]
                                .map(|n| Tag::Double(n.to_bits()))
                                .to_vec(),
                        ),
                    ),
                    ("block_pos", Tag::IntArray(vec![1, 70, 3])),
                    (
                        "Item",
                        Tag::compound([
                            ("id", Tag::String("minecraft:stone".into())),
                            ("count", Tag::Int(1)),
                        ]),
                    ),
                    (
                        sunrise_mod_runtime::attachments::ATTACHMENTS_KEY,
                        Tag::compound([(
                            "saved_entity",
                            Tag::compound([(
                                "value",
                                Tag::compound([("value", Tag::Compound(Default::default()))]),
                            )]),
                        )]),
                    ),
                ]);
                match kind {
                    "item" => game.load_item(tag, Some(uuid)),
                    "experience_orb" => game.load_orb(tag, Some(uuid)),
                    "armor_stand" => game.load_armor_stand(tag, Some(uuid)),
                    "item_frame" => game.load_item_frame(tag, Some(uuid)),
                    "oak_boat" => game.load_boat(tag, Some(uuid), "minecraft:oak_boat"),
                    "minecart" => game.load_cart(tag, Some(uuid), "minecraft:minecart"),
                    "arrow" => game.load_arrow(tag, Some(uuid), None),
                    "small_fireball" => game.load_fireball(tag, Some(uuid), None),
                    "snowball" => game.load_throwable(tag, Some(uuid), None),
                    _ => unreachable!(),
                }
                let key = ("saved_entity".into(), "value".into());
                let index = game.attachment_indexes[&Dim::Overworld].clone();
                let saved = index.lock().unwrap().snapshot(&key);
                assert_eq!(saved.len(), 1, "{kind} before typed access");
                let AttachmentHolder::Entity(entity) = saved[0].holder else {
                    panic!("not entity");
                };
                assert_ne!(entity.id(), 0, "{kind}");
                value_key
                    .set(game, Holder::Entity(entity.id()), 19)
                    .unwrap();
                let old = game.query_mut(value_key).unwrap();
                let row = old.iter().try_next().unwrap().unwrap();
                let work = index.lock().unwrap().work_count();
                // Exercise the clone-replay boundary used by vehicle/projectile ticks.
                match kind {
                    "oak_boat" => {
                        let copy = game.boats[0].clone();
                        assert!(game.boats.store_back(0, copy));
                    }
                    "minecart" => {
                        let copy = game.carts[0].clone();
                        assert!(game.carts.store_back(0, copy));
                    }
                    "arrow" | "small_fireball" | "snowball" => {
                        let copy = game.creatures.projectiles[0].clone();
                        assert!(game.creatures.projectiles.store_back(0, copy));
                    }
                    _ => (),
                }
                assert_eq!(
                    index.lock().unwrap().work_count(),
                    work,
                    "{kind} store_back"
                );
                assert_eq!(
                    value_key.get(game, Holder::Entity(entity.id()), Clone::clone),
                    Ok(19),
                    "{kind}"
                );
                game.enter_level(Dim::Nether).unwrap();
                assert_eq!(
                    index.lock().unwrap().snapshot(&key).len(),
                    1,
                    "{kind} parked"
                );
                game.enter_level(Dim::Overworld).unwrap();
                assert!(game.entity_discard(entity), "{kind}");
                game.native_removals_flush();
                assert!(
                    index.lock().unwrap().snapshot(&key).is_empty(),
                    "{kind} sweep"
                );
                assert_eq!(
                    row.set(game, 20),
                    Err(mod_api::AttachmentRowError::StaleHolder),
                    "{kind}"
                );
                assert_eq!(
                    row.with_value(game.attachment_frame_borrow(), |value| *value)
                        .unwrap(),
                    19
                );
            }
        },
    );
}

/// The saved-binding families whose first store used to stay unindexed (#995): each publishes
/// exactly its own row, and the removal sweep retires it. Mobs, paintings, dragon parts and
/// general rows already have their own first-store tests.
#[test]
#[ignore = "daily"]
fn saved_binding_family_first_stores_publish_to_queries() {
    with_attachment_game(
        "saved_entity",
        saved_entity_descriptor,
        |game, value_key| {
            sunrise_gameplay_common::game::test_fixtures::fixture_resident_column(
                &mut game.world,
                (0, 0),
            );
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(Dim::Overworld);
            let index = game.attachment_indexes[&Dim::Overworld].clone();
            let key = ("saved_entity".into(), "value".into());
            macro_rules! family {
            ($kind:literal, $nbt:literal, $($field:ident).+) => {{
                game.summon_with_nbt(concat!("minecraft:", $kind), [0.5, 70.0, 0.5], $nbt)
                    .unwrap();
                game.lan_assign_ids();
                let id = game$(.$field)+.last().unwrap().id;
                let holder = Holder::Entity(id);
                let entity = game.entity(id).unwrap();
                assert!(
                    index.lock().unwrap().snapshot(&key).is_empty(),
                    "{} publishes before a store",
                    $kind
                );
                value_key.set(game, holder, 7).unwrap();
                {
                    let query = game.query(value_key).unwrap();
                    let mut iter = query.iter();
                    let rows: Vec<_> = std::iter::from_fn(|| iter.try_next().unwrap()).collect();
                    assert_eq!(rows.len(), 1, "{} first store", $kind);
                    assert_eq!(rows[0].holder, AttachmentHolder::Entity(entity), "{}", $kind);
                    assert_eq!(rows[0].with_value(game.attachment_frame_borrow(), |value| *value), Ok(7), "{}", $kind);
                }
                assert!(game.entity_discard(entity), "{} discard", $kind);
                game.native_removals_flush();
                assert!(
                    index.lock().unwrap().snapshot(&key).is_empty(),
                    "{} sweep",
                    $kind
                );
            }};
        }
            family!("item", r#"{Item:{id:"minecraft:stone",count:1}}"#, dropped);
            family!("experience_orb", "{Value:1s}", orbs);
            family!("arrow", "{}", creatures.projectiles);
            family!("armor_stand", "{}", armor_stands);
            family!("item_frame", "{Facing:3b}", item_frames);
            family!("oak_boat", "{}", boats);
            family!("minecart", "{}", carts);
            // The gate stays narrow: a family without saved binding stores without publishing.
            game.summon_with_nbt("minecraft:tnt", [0.5, 70.0, 0.5], "{}")
                .unwrap();
            game.lan_assign_ids();
            let id = game.tnts[0].id;
            value_key.set(game, Holder::Entity(id), 9).unwrap();
            assert!(
                game.query(value_key)
                    .unwrap()
                    .iter()
                    .try_next()
                    .unwrap()
                    .is_none(),
                "unindexed family"
            );
        },
    );
}
