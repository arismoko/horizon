use super::super::native_entity_refs::tests::with_game;
use mod_api::DimensionKey;
use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
};
use sunrise_gameplay::game::server_side::entity_residency::SlotAccess;
use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
use sunrise_items::items::ItemStack;
use sunrise_mod_runtime::attachments::{
    AttachmentType, Attachments, Holder, index::AttachmentIndex,
};
use sunrise_sim::join_seq::residency::{Kind, table::EntityLookup};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "all_resident_families",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

#[test]
#[ignore = "daily"]
fn every_ordinary_resident_family_binds_its_actual_owned_slot() {
    with_game(|game| {
        // A handle resolves only for an entity in an accessible section
        // (`LevelEntityGetter`); every entity here stands in chunk (0, 0).
        fixture_resident_column(&mut game.world, (0, 0));
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let key = ("test".into(), "all_resident_families".into());
        let mut maps = Vec::new();
        let mut bind =
            |game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, expected: Kind| {
                let EntityLookup::Found { location, .. } = game.server_resident_entity(id) else {
                    panic!("{expected:?} did not become resident");
                };
                assert_eq!(location.owner.kind, expected);
                let holder = game.entity(id).unwrap();
                let mut map = Attachments::default();
                VALUE.set(&mut map, Holder::Entity(id), 31).unwrap();
                assert_eq!(
                    game.with_resident_slot(
                        &DimensionKey::OVERWORLD,
                        NonZeroU32::new(id).unwrap(),
                        |slot| {
                            assert!(!slot.is_bound(), "{expected:?} was already bound");
                            slot.bind(&mut map, &index, holder);
                            slot.is_bound()
                        }
                    ),
                    SlotAccess::Done(true)
                );
                maps.push(map);
                assert_eq!(
                    index.lock().unwrap().snapshot(&key).len(),
                    maps.len(),
                    "{expected:?} failed to publish"
                );
            };
        macro_rules! summon {
            ($name:literal, $kind:ident, $($field:tt).+) => {{
                game.summon_with_nbt(concat!("minecraft:", $name), [0.5, 70.0, 0.5], "{}").unwrap();
                game.lan_assign_ids();
                let id = game$(.$field)+[0].id;
                bind(game, id, Kind::$kind);
            }};
        }
        summon!("pig", Mob, mobs);
        game.summon_with_nbt(
            "minecraft:item",
            [0.5, 70.0, 0.5],
            "{Item:{id:\"minecraft:stone\",count:1}}",
        )
        .unwrap();
        game.lan_assign_ids();
        let id = game.dropped[0].id;
        bind(game, id, Kind::Item);
        summon!("experience_orb", Orb, orbs);
        summon!("snowball", Projectile, creatures.projectiles);
        summon!("armor_stand", Stand, armor_stands);
        summon!("item_frame", Frame, item_frames);
        summon!("painting", Painting, paintings);
        summon!("oak_boat", Boat, boats);
        summon!("minecart", Cart, carts);
        summon!("marker", Other, server.others.0);
        game.add_fangs([0.5, 70.0, 0.5], 0.0, 0, 0);
        let id = game.fangs.list[0].id;
        bind(game, id, Kind::Fangs);
        summon!("end_crystal", Crystal, crystals);
        summon!("tnt", Tnt, tnts);
        summon!("falling_block", Falling, falling);
        summon!("firework_rocket", Rocket, rockets);
        summon!("trident", Trident, tridents);
        game.add_ominous_item_spawner(ItemStack::new("minecraft:stone", 1), [0.5, 70.0, 0.5], 60);
        let id = game.ominous_spawners[0].id;
        bind(game, id, Kind::Spawner);
        summon!("area_effect_cloud", Cloud, breath_clouds);
        summon!("lightning_bolt", Bolt, bolts);
        summon!("cushion", Cushion, cushions);
        let fence = [0, 70, 0];
        game.tie_knot(fence);
        let mob_id = game.mobs[0].id;
        assert!(game.set_leash(
            sunrise_gameplay_common::game::commands_common::Target::Mob(mob_id),
            sunrise_sim::mobs::leash::Holder::Knot(fence),
            None,
        ));
        let id = game.resident_entities.knots[0].id;
        bind(game, id, Kind::Knot);
        game.summon_with_nbt("minecraft:ender_dragon", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        let id = game.dragon.as_ref().unwrap().id;
        bind(game, id, Kind::Dragon);
        // #792: a general row joins its own family's list; the lookup is the
        // same table path every family takes.
        game.resident_entities.general.rows.push(
            sunrise_gameplay_common::game::general::GeneralRow {
                id: 4242,
                uuid: [11, 12, 13, 14],
                join: sunrise_sim::join_seq::JoinSeq::UNJOINED,
                type_slot: 0,
                position: sunrise_core::geometry::Vec3::ZERO,
                previous: sunrise_core::geometry::Vec3::ZERO,
                velocity: sunrise_core::geometry::Vec3::ZERO,
                rotation: [0.0, 0.0],
                previous_rotation: [0.0, 0.0],
                owner: None,
                fall_distance: 0.0,
                custom_name: None,
                custom_name_visible: false,
                base: Default::default(),
                on_ground: false,
                tick_count: 0,
                no_physics: false,
                horizontal_collision: false,
                vertical_collision: false,
                vertical_collision_below: false,
                fluids: Default::default(),
                under_water: false,
                payload_version: 1,
                payload_bytes: Vec::new(),
                mount: None,
                vehicle_order: 0,
                boarding_cooldown: 0,
                attachment_lifetime: Default::default(),
            },
        );
        bind(game, 4242, Kind::General);
        assert_eq!(maps.len(), 23);
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 23);
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
