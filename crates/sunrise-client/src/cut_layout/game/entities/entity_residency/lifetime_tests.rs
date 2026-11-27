use mod_api::DimensionKey;
use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
};
use sunrise_items::items::ItemStack;
use sunrise_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use sunrise_sim::join_seq::RestoreOrder;
use {
    sunrise_gameplay::game::entity_residency::SlotAccess,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "resident_family_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

// All cases use one resource graph. Keep both the external map and the old
// published batch alive when dropping the actual owner: neither may retain
// registration authority. Moving the actual owner out and back must retain it.
macro_rules! check_list {
    ($game:ident, $($field:tt).+) => {{
        let id = $game$(.$field)+[0].id;
        let holder = $game.entity(id).expect(stringify!($($field).+));
        let id = NonZeroU32::new(id).unwrap();
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let key = ("test".into(), "resident_family_lifetime".into());
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(id.get()), 31).unwrap();
        let result = $game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| {
            slot.bind(&mut values, &index, holder);
            slot.registration().unwrap()
        });
        let SlotAccess::Done(registration) = result else { panic!("{} unavailable", stringify!($($field).+)) };
        $game$(.$field)+.update(0, |owner| assert!(registration.same_registration(&owner.attachment_lifetime().registration().unwrap())));
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1, "{} did not publish", stringify!($($field).+));
        let lifted = $game$(.$field)+.lift();
        assert_eq!($game.with_resident_slot(&DimensionKey::OVERWORLD, id, |_| ()), SlotAccess::Taken);
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        $game$(.$field)+.restore(lifted, RestoreOrder::SurvivorsFirst);
        assert_eq!($game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| registration.same_registration(&slot.registration().unwrap())), SlotAccess::Done(true));
        $game$(.$field)+.clear();
        assert!(index.lock().unwrap().snapshot(&key).is_empty(), "{} registration survived actual owner drop", stringify!($($field).+));
        assert_eq!(before.len(), 1);
        assert_eq!($game.with_resident_slot(&DimensionKey::OVERWORLD, id, |_| ()), SlotAccess::Missing);
        VALUE.set(&mut values, Holder::Entity(id.get()), 32).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
    }};
}

#[test]
#[ignore = "daily"]
fn resident_new_family_lifetimes_bind_move_and_retire() {
    with_game(|game| {
        sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            sunrise_core::position::ChunkPos::new(0, 0),
        );
        game.summon_with_nbt("minecraft:marker", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        check_list!(game, server.others.0);

        game.add_fangs([0.5, 70.0, 0.5], 0.0, 0, 0);
        check_list!(game, fangs.list);

        game.summon_with_nbt("minecraft:end_crystal", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        check_list!(game, crystals);

        game.summon_with_nbt("minecraft:tnt", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        check_list!(game, tnts);

        game.server.next_entity += 1;
        let id = game.server.next_entity;
        game.resident_entities
            .knots
            .push(sunrise_gameplay_common::leash_knot::Knot {
                attachment_lifetime: Default::default(),
                id,
                join: Default::default(),
                fence: [0, 70, 0],
                fire_ticks: 0,
                invulnerable: false,
                no_gravity: false,
                silent: false,
                glowing: false,
            });
        check_list!(game, resident_entities.knots);

        game.summon_with_nbt(
            "minecraft:firework_rocket",
            [0.5, 70.0, 0.5],
            "{LifeTime:20}",
        )
        .unwrap();
        game.lan_assign_ids();
        check_list!(game, rockets);

        game.summon_with_nbt("minecraft:trident", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        check_list!(game, tridents);

        game.add_ominous_item_spawner(ItemStack::new("minecraft:stone", 1), [0.5, 70.0, 0.5], 60);
        check_list!(game, ominous_spawners);

        game.summon_with_nbt("minecraft:area_effect_cloud", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        check_list!(game, breath_clouds);

        game.summon_with_nbt("minecraft:ender_dragon", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        let id = NonZeroU32::new(game.dragon.as_ref().unwrap().id).unwrap();
        let holder = game.entity(id.get()).unwrap();
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let key = ("test".into(), "resident_family_lifetime".into());
        let mut values = Attachments::default();
        VALUE
            .set(&mut values, Holder::Entity(id.get()), 41)
            .unwrap();
        let SlotAccess::Done(registration) =
            game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| {
                slot.bind(&mut values, &index, holder);
                slot.registration().unwrap()
            })
        else {
            panic!("whole dragon has no actual slot")
        };
        assert!(
            registration.same_registration(
                &game
                    .dragon
                    .as_mut()
                    .unwrap()
                    .attachment_lifetime()
                    .registration()
                    .unwrap()
            )
        );
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        let dragon = game.dragon.extract().unwrap();
        assert_eq!(
            game.with_resident_slot(&DimensionKey::OVERWORLD, id, |_| ()),
            SlotAccess::Taken
        );
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.dragon.put_back(dragon);
        assert_eq!(
            game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| registration
                .same_registration(&slot.registration().unwrap())),
            SlotAccess::Done(true)
        );
        game.dragon.discard();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before.len(), 1);
        assert_eq!(
            game.with_resident_slot(&DimensionKey::OVERWORLD, id, |_| ()),
            SlotAccess::Missing
        );
        VALUE
            .set(&mut values, Holder::Entity(id.get()), 42)
            .unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
