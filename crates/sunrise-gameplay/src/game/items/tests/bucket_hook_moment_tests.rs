//! `BucketItem.use` runs one branch per click: an empty bucket picks up
//! (`BucketPickup.pickupBlock`), a full one empties (`emptyContents`). A mod
//! hears the pickup through `fill_bucket` and the pour through `bucket_empty`,
//! each only on its own branch.
use super::*;
use mod_api::{BucketEmptyEvent, FillBucketEvent, Flow, Hooks, ModEntry, Mods, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest;
use sunrise_level::chunk::Chunk;

/// The hooks each click was heard by, in order.
#[derive(Clone, Default)]
struct Heard(Arc<Mutex<Vec<&'static str>>>);

#[mod_api::__subscriptions]
impl Hooks for Heard {
    fn fill_bucket(&self, _: &mut dyn ServerHost, _: &mut FillBucketEvent<'_>) -> Flow {
        self.0.lock().unwrap().push("fill_bucket");
        Flow::Continue
    }
    fn bucket_empty(&self, _: &mut dyn ServerHost, _: &mut BucketEmptyEvent<'_>) -> Flow {
        self.0.lock().unwrap().push("bucket_empty");
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_pour_is_heard_by_bucket_empty_alone_and_a_pickup_by_fill_bucket_alone() {
    let heard = Heard::default();
    let mods: Mods = vec![ModEntry::new("heard", Box::new(heard.clone()))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            crate::game::native_item_places::tests::open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (stone, water) = (state("minecraft:stone"), state("minecraft:water"));
            game.world.set(6, 63, 5, stone).unwrap();
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            game.as_guest(1, |game| {
                game.creative = false;
                // A water bucket on the stone's top pours into the air above it.
                let full = ItemStack::new("minecraft:water_bucket", 1);
                *game.held_mut() = Some(full.clone());
                game.server.forced_hit = Some(Hit {
                    pos: [6, 63, 5],
                    face: [0, 1, 0],
                    point: [6.5, 64.0, 5.5],
                    inside: false,
                });
                let poured = game.use_tool(&full, true).unwrap();
                assert!(matches!(
                    poured,
                    sunrise_sim::first_person_sim::UseResult::Success { .. }
                ));
                assert_eq!(game.world.block(6, 64, 5), water);
                assert_eq!(*heard.0.lock().unwrap(), ["bucket_empty"]);
                heard.0.lock().unwrap().clear();

                // The empty bucket it leaves picks the source back up.
                let empty = ItemStack::new("minecraft:bucket", 1);
                assert_eq!(game.held(), Some(&empty));
                game.server.forced_hit = Some(Hit {
                    pos: [6, 64, 5],
                    face: [0, 1, 0],
                    point: [6.5, 65.0, 5.5],
                    inside: false,
                });
                let picked = game.use_tool(&empty, true).unwrap();
                game.server.forced_hit = None;
                assert!(matches!(
                    picked,
                    sunrise_sim::first_person_sim::UseResult::Success { .. }
                ));
                assert_eq!(game.world.block(6, 64, 5), state("minecraft:air"));
                assert_eq!(*heard.0.lock().unwrap(), ["fill_bucket"]);
            })
            .unwrap();
        });
    });
}
