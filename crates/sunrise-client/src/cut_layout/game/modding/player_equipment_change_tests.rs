//! `living_equipment_change` for a server's players and mobs (#3417, debt p3-39):
//! borrow-based comparison without blanket equipment cloning, all eight
//! `EquipmentSlot`s matching vanilla `LivingEntity.collectEquipmentChanges` and
//! `PlayerEquipment`, lazy `ItemStackView` delivery, and minimal history retention.
use std::sync::{Arc, Mutex};

use mod_api::{self, EquipmentSlot, Hooks, LivingEquipmentChangeEvent, ServerHost};
use sunrise_core::geometry::Vec3;
use sunrise_items::equipment::{BODY, CHEST, SADDLE};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::{
    lan::guest::SPECTATOR,
    mobs::{Mob, finalize},
};

use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

/// What each call saw: the entity id, its resolved kind, the slot, and projected stacks.
type Heard = Vec<(
    u32,
    Option<String>,
    EquipmentSlot,
    Option<mod_api::ItemStack>,
    Option<mod_api::ItemStack>,
)>;

struct Counter {
    heard: Arc<Mutex<Heard>>,
}
#[mod_api::__subscriptions]
impl Hooks for Counter {
    fn living_equipment_change(
        &self,
        host: &mut dyn ServerHost,
        event: &mut LivingEquipmentChangeEvent<'_>,
    ) {
        let kind = event
            .entity
            .entity()
            .kind(host)
            .and_then(|kind| kind.id(host))
            .map(|id| id.as_str().to_owned());
        self.heard.lock().unwrap().push((
            event.entity.id(),
            kind,
            event.slot,
            event.from.as_ref().and_then(|stack| stack.project()),
            event.to.as_ref().and_then(|stack| stack.project()),
        ));
    }
}

struct Quiet;
#[mod_api::__subscriptions]
impl Hooks for Quiet {}

fn with_counter(
    listens: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Mutex<Heard>),
) {
    let heard = Arc::new(Mutex::new(Heard::new()));
    let entry = if listens {
        mod_api::ModEntry::new(
            "counter",
            Box::new(Counter {
                heard: heard.clone(),
            }),
        )
    } else {
        mod_api::ModEntry::new("quiet", Box::new(Quiet))
    };
    let mods: mod_api::Mods = vec![entry].into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // A joined guest has its play state from the login on.
            let host = game.server.lan.as_mut().unwrap();
            for conn in [12, 13] {
                host.play.entry(conn).or_default();
            }
            test(game, &heard)
        })
    });
}

fn api(stack: &ItemStack) -> Option<mod_api::ItemStack> {
    Some(stack.try_into_api().unwrap())
}

fn take(heard: &Mutex<Heard>) -> Heard {
    std::mem::take(&mut *heard.lock().unwrap())
}

fn player(
    slot: EquipmentSlot,
    from: Option<mod_api::ItemStack>,
    to: Option<mod_api::ItemStack>,
) -> Heard {
    vec![(12, Some("minecraft:player".into()), slot, from, to)]
}

#[test]
#[ignore = "daily"]
fn a_players_hotbar_selection_pickup_and_helmet_are_each_one_change() {
    with_counter(true, |game, heard| {
        let sword = ItemStack::new("minecraft:iron_sword", 1);
        let stone = ItemStack::new("minecraft:stone", 1);
        let helmet = ItemStack::new("minecraft:iron_helmet", 1);
        let body = ItemStack::new("minecraft:diamond_horse_armor", 1);
        let saddle = ItemStack::new("minecraft:saddle", 1);
        // Both guests start empty-handed in slot 0: counting from empty, nothing changed
        // and no history table is allocated.
        game.tick_guests();
        assert_eq!(take(heard), Heard::new());
        assert!(
            game.server.lan.as_ref().unwrap().play[&12]
                .last_equipment
                .is_none()
        );

        // Selecting hotbar slot 1, which holds a sword, moves it into the main hand.
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.inventory[1] = Some(sword.clone());
            g.selected = 1;
        });
        game.tick_guests();
        assert_eq!(
            take(heard),
            player(EquipmentSlot::MainHand, None, api(&sword))
        );

        // Back to the empty slot 0, then a stone picked up into it.
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| g.selected = 0);
        game.tick_guests();
        assert_eq!(
            take(heard),
            player(EquipmentSlot::MainHand, api(&sword), None)
        );
        assert!(
            game.server.lan.as_ref().unwrap().play[&12]
                .last_equipment
                .is_none()
        );

        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| g.inventory[0] = Some(stone.clone()));
        game.tick_guests();
        assert_eq!(
            take(heard),
            player(EquipmentSlot::MainHand, None, api(&stone))
        );

        // A helmet put on from the inventory screen (`armor` runs head to feet).
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| g.armor[0] = Some(helmet.clone()));
        game.tick_guests();
        assert_eq!(take(heard), player(EquipmentSlot::Head, None, api(&helmet)));

        // Body and saddle slots follow vanilla `PlayerEquipment` / `EquipmentSlot.VALUES`.
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.equipment[BODY] = Some(body.clone());
            g.equipment[SADDLE] = Some(saddle.clone());
        });
        game.tick_guests();
        assert_eq!(
            take(heard),
            vec![
                (
                    12,
                    Some("minecraft:player".into()),
                    EquipmentSlot::Body,
                    None,
                    api(&body)
                ),
                (
                    12,
                    Some("minecraft:player".into()),
                    EquipmentSlot::Saddle,
                    None,
                    api(&saddle)
                ),
            ]
        );

        // A tick with no change tells nothing.
        game.tick_guests();
        assert_eq!(take(heard), Heard::new());
    });
}

#[test]
#[ignore = "daily"]
fn item_use_completion_reports_equipment_change_in_same_server_tick() {
    with_counter(true, |game, heard| {
        let apple = ItemStack::new("minecraft:golden_apple", 1);
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.inventory[0] = Some(apple.clone());
            g.selected = 0;
        });
        game.tick_guests();
        assert_eq!(
            take(heard),
            player(EquipmentSlot::MainHand, None, api(&apple))
        );

        let duration = game
            .as_guest(12, |g| {
                assert!(g.start_using_item());
                g.use_duration
            })
            .unwrap();
        assert_eq!(duration, 32);

        for _ in 0..(duration - 1) {
            game.tick_guests();
        }
        assert_eq!(take(heard), Heard::new());
        assert!(game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0].is_some());

        // On the final use tick, `tick_using` finishes consuming the apple and
        // `detect_guest_equipment_updates` reports the emptied MainHand in the
        // same server tick, before dropping the empty history box.
        game.tick_guests();
        assert!(game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0].is_none());
        assert_eq!(
            take(heard),
            player(EquipmentSlot::MainHand, api(&apple), None)
        );
        assert!(
            game.server.lan.as_ref().unwrap().play[&12]
                .last_equipment
                .is_none()
        );

        game.tick_guests();
        assert_eq!(take(heard), Heard::new());
    });
}

#[test]
#[ignore = "daily"]
fn spectator_touching_unloaded_chunk_skips_equipment_change_hook() {
    with_counter(true, |game, heard| {
        let helmet = ItemStack::new("minecraft:iron_helmet", 1);
        game.world.chunks.clear();
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.game_mode = SPECTATOR;
            g.position = [8.5, 64.0, 8.5];
            g.armor[0] = Some(helmet.clone());
        });

        // Spectator touching an unloaded chunk skips `ServerPlayer.doTick`'s
        // body (`super.tick()` -> `detectEquipmentUpdates`).
        game.tick_guests();
        assert_eq!(take(heard), Heard::new());
        assert!(
            game.server.lan.as_ref().unwrap().play[&12]
                .last_equipment
                .is_none()
        );

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.tick_guests();
        assert_eq!(take(heard), player(EquipmentSlot::Head, None, api(&helmet)));
        assert!(
            game.server.lan.as_ref().unwrap().play[&12]
                .last_equipment
                .is_some()
        );
    });
}

#[test]
#[ignore = "daily"]
fn players_advance_equipment_history_without_a_subscriber() {
    with_counter(false, |game, heard| {
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.inventory[1] = Some(ItemStack::new("minecraft:iron_sword", 1));
            g.selected = 1;
            g.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
        });
        game.tick_guests();
        assert_eq!(take(heard), Heard::new());
        assert!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .play
                .get(&12)
                .unwrap()
                .last_equipment
                .is_some()
        );
    });
}

#[test]
#[ignore = "daily"]
fn mobs_compare_equipment_by_borrow_and_retain_only_changed_slots() {
    if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
        "cut_layout::game::player_equipment_change_tests::mobs_compare_equipment_by_borrow_and_retain_only_changed_slots",
        "SUNRISE_MOB_EQUIPMENT_HISTORY_CASE",
        &["toggle"],
    ) {
        return;
    }
    with_counter(true, |game, heard| {
        use sunrise_sim::mobs::armor;

        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        for (id, x) in [(41, 3.0), (42, 6.0)] {
            let mut mob = Mob::new(id, zombie.clone(), Vec3::new(x, 64.0, 5.0), 0.0);
            mob.no_ai = true;
            mob.no_gravity = true;
            game.mobs.push(mob);
        }

        // Naked mobs emit nothing and allocate no equipment history.
        game.tick_creature_entities().unwrap();
        assert_eq!(take(heard), Heard::new());
        assert!(!game.mobs[0].extra.live.has_worn());
        assert!(!game.mobs[1].extra.live.has_worn());
        let bare = armor::values(&game.mobs[1]);

        // Equip mob 42 with a damaged enchanted chestplate (gear overlay on stored stack).
        let mut chest = ItemStack::new("minecraft:iron_chestplate", 1);
        chest.damage = 3;
        chest.set_crafting_enchantments(vec![(Arc::from("minecraft:protection"), 2)]);
        finalize::set_item_stack(&mut game.mobs[1], CHEST, chest.clone());

        game.tick_creature_entities().unwrap();
        assert_eq!(
            take(heard),
            vec![(
                42,
                Some("minecraft:zombie".into()),
                EquipmentSlot::Chest,
                None,
                api(&chest),
            )]
        );
        // Unchanged mob 41 still allocates no history; mob 42 retains only chest.
        assert!(!game.mobs[0].extra.live.has_worn());
        assert!(game.mobs[1].extra.live.has_worn());
        let wearing_iron = armor::values(&game.mobs[1]);
        assert!(wearing_iron.0 > bare.0);

        // An unchanged tick with overlaid gear reports nothing.
        game.tick_creature_entities().unwrap();
        assert_eq!(take(heard), Heard::new());

        // The same comparison still refreshes armor and advances history
        // when the subscriber is disabled. Enabling it does not replay that change.
        let diamond = ItemStack::new("minecraft:diamond_chestplate", 1);
        assert!(mod_api::__private::set_mod_enabled("counter", false));
        game.mobs[1].set_equipment_stack(CHEST, Some(diamond.clone()));
        game.tick_creature_entities().unwrap();
        assert_eq!(take(heard), Heard::new());
        assert!(armor::values(&game.mobs[1]).0 > wearing_iron.0);
        assert!(mod_api::__private::set_mod_enabled("counter", true));
        game.tick_creature_entities().unwrap();
        assert_eq!(take(heard), Heard::new());

        // A handle write applies armor immediately without consuming the
        // body's next notification; its previous value is the silent change.
        let mob = game.living(game.entity(42).unwrap()).unwrap();
        assert!(game.living_set_equipment(mob, EquipmentSlot::Chest, None));
        assert_eq!(armor::values(&game.mobs[1]), bare);
        game.tick_creature_entities().unwrap();
        assert_eq!(
            take(heard),
            vec![(
                42,
                Some("minecraft:zombie".into()),
                EquipmentSlot::Chest,
                api(&diamond),
                None,
            )]
        );
        assert!(!game.mobs[1].extra.live.has_worn());
    });
}
