use mod_api::ServerHost;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Mob;

/// `LivingDropsEvent` hears a copper golem's preserved equipment with the
/// rest of its drops and can cancel it, and an armor stand's break
/// fires it with an empty list.
#[test]
#[ignore = "daily"]
fn the_golems_preserved_equipment_and_a_broken_stand_reach_living_drops() {
    use mod_api::{self, Flow, Hooks, LivingDropsEvent};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Observer(Arc<Mutex<Vec<String>>>, Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn living_drops(&self, _: &mut dyn ServerHost, event: &mut LivingDropsEvent<'_>) -> Flow {
            let items: Vec<_> = event
                .drops
                .iter()
                .map(|d| {
                    mod_api::__private::item_key_id(d.stack.item())
                        .unwrap()
                        .to_string()
                })
                .collect();
            self.0.lock().unwrap().push(format!(
                "{} {:?} {} {}",
                event.kind, items, event.source.damage_type, event.recently_hit
            ));
            if self.1.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = Arc::from(vec![mod_api::ModEntry::new(
        "living-drops-observer",
        Box::new(Observer(seen.clone(), cancel.clone())),
    )]);
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.load_server_loot();
            let golem_type = game
                .res
                .mob_types()
                .get("minecraft:copper_golem")
                .unwrap()
                .clone();
            let spawn_golem = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.server.next_entity += 1;
                let mut golem = Mob::new(
                    game.server.next_entity,
                    golem_type.clone(),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                sunrise_sim::mobs::transport_items::set_held(
                    &mut golem,
                    Some(ItemStack::new("minecraft:diamond", 1)),
                );
                golem.health = 0.0;
                golem
            };
            let golem = spawn_golem(game);
            game.mobs.push(golem.clone());
            game.mob_die(&golem);
            // The preserved stack comes last in the list the mods hear.
            assert!(
                seen.lock().unwrap()[0].starts_with("minecraft:copper_golem [")
                    && seen.lock().unwrap()[0]
                        .ends_with("\"minecraft:diamond\"] minecraft:generic false")
            );
            let dropped = game.dropped.len();
            assert!(dropped >= 1);
            // A canceled list drops nothing, the preserved stack included.
            cancel.store(true, Ordering::SeqCst);
            let golem = spawn_golem(game);
            game.mob_die(&golem);
            assert_eq!(game.dropped.len(), dropped);
            // An armor stand that breaks fires it with nothing in the list.
            game.armor_stands
                .push(sunrise_sim::entities::ArmorStand::new(
                    9000,
                    Vec3::new(4.5, 64.0, 4.5),
                    0.0,
                ));
            game.break_armor_stand_by_anything(0, "minecraft:player_explosion", 12);
            let seen = seen.lock().unwrap();
            assert_eq!(
                seen.last().unwrap(),
                "minecraft:armor_stand [] minecraft:player_explosion false"
            );
        });
    });
}
