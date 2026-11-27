#![cfg(test)]
//! `ItemPart::damage_item` on the wear of a player's held tool, and the part the tests of the other wear
//! paths (the dispenser, the shield, the mace) share. Each installs process-wide parts: run alone with the
//! prepared resources.
use mod_api::{DamageItemEvent, Flow, Hooks, ItemPart, ModEntry, Registrar, RegistrationContext};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
use sunrise_items::items::ItemStack;

/// What the part of an item does to the wear it is asked about.
#[derive(Clone, Copy)]
pub(crate) enum Rule {
    /// Half the points, rounded down.
    Halve,
    /// This many times the points.
    Times(i32),
    /// Replace the wear, including boundary values.
    Set(i32),
    /// No wear at all.
    Cancel,
    /// Break the item now (`break_now`), whatever durability it has left.
    Break,
}

/// The part of one item: it applies its `rule` and records the points it was asked about.
#[derive(Clone)]
pub(crate) struct Wear {
    item: &'static str,
    pub(in crate::game) rule: Arc<Mutex<Rule>>,
    pub(in crate::game) asked: Arc<Mutex<Vec<i32>>>,
    /// The entity id each wear named (`event.entity`).
    worn_by: Arc<Mutex<Vec<Option<u32>>>>,
}

#[mod_api::sunrise_part]
impl ItemPart for Wear {
    fn damage_item(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut DamageItemEvent<'_>,
    ) -> Flow {
        self.asked.lock().unwrap().push(event.amount);
        let entity = event.entity.map(mod_api::LivingRef::id);
        self.worn_by.lock().unwrap().push(entity);
        match *self.rule.lock().unwrap() {
            Rule::Halve => event.amount /= 2,
            Rule::Times(factor) => event.amount *= factor,
            Rule::Set(amount) => event.amount = amount,
            Rule::Cancel => return Flow::Cancel,
            Rule::Break => event.break_now = true,
        }
        Flow::Continue
    }
}

struct Mod(Wear);
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key(self.0.item);
        r.item_part(key, self.0.clone());
    }
}

impl Wear {
    /// The points asked about since the last call.
    pub(crate) fn take_asked(&self) -> Vec<i32> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
    /// The entities the wears since the last call named.
    pub(crate) fn take_worn_by(&self) -> Vec<Option<u32>> {
        std::mem::take(&mut *self.worn_by.lock().unwrap())
    }
    pub(crate) fn set(&self, rule: Rule) {
        *self.rule.lock().unwrap() = rule;
    }
}

/// Runs `test` on a serving game whose `item` has a part, starting with `rule`.
pub(crate) fn with_wear_part(
    item: &'static str,
    rule: Rule,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Wear),
) {
    let wear = Wear {
        item,
        rule: Arc::new(Mutex::new(rule)),
        asked: Arc::default(),
        worn_by: Arc::default(),
    };
    let mods: mod_api::Mods = vec![ModEntry::new("wear", Box::new(Mod(wear.clone())))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| test(game, &wear));
    });
}

/// Uses `tool` once for `points` on the player, and the damage it has afterwards.
fn used(game: &mut sunrise_gameplay::game::ServerGame<'_>, tool: &str, points: u32) -> u32 {
    game.creative = false;
    game.selected = 0;
    game.inventory[0] = Some(ItemStack::new(tool, 1));
    game.damage_held(points);
    game.inventory[0]
        .as_ref()
        .map_or(u32::MAX, |stack| stack.damage)
}

/// A part that halves the wear makes its tool lose half the durability per use, a part that cancels
/// it stops the wear, and a tool without a part wears as vanilla and is never asked.
#[test]
#[ignore = "daily"]
fn a_part_changes_the_wear_of_its_own_tool_on_the_player() {
    with_wear_part("minecraft:iron_pickaxe", Rule::Halve, |game, wear| {
        assert_eq!(used(game, "minecraft:iron_pickaxe", 10), 5);
        assert_eq!(wear.take_asked(), [10]);
        // 7 points are halved to 3, as the integer halving a part does.
        assert_eq!(used(game, "minecraft:iron_pickaxe", 7), 3);
        assert_eq!(wear.take_asked(), [7]);

        wear.set(Rule::Cancel);
        assert_eq!(used(game, "minecraft:iron_pickaxe", 10), 0);
        assert_eq!(wear.take_asked(), [10]);

        // A pickaxe of another material has no part: no stack conversion or dispatch.
        let dispatches = game.damage_item_dispatches();
        assert_eq!(used(game, "minecraft:diamond_pickaxe", 10), 10);
        assert!(wear.take_asked().is_empty());
        assert_eq!(game.damage_item_dispatches(), dispatches);

        // A creative player's tools take no wear, so the part is not asked.
        game.creative = true;
        game.inventory[0] = Some(ItemStack::new("minecraft:iron_pickaxe", 1));
        game.damage_held(10);
        assert_eq!(game.inventory[0].as_ref().unwrap().damage, 0);
        assert!(wear.take_asked().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn part_amount_boundaries_and_threshold_crossing() {
    with_wear_part("minecraft:iron_pickaxe", Rule::Set(0), |game, wear| {
        for amount in [i32::MIN, -1, 0] {
            wear.set(Rule::Set(amount));
            assert_eq!(used(game, "minecraft:iron_pickaxe", 1), 0);
            assert_eq!(wear.take_asked(), [1]);
        }
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0),
            )
            .unwrap(),
        );
        game.server.lan_acting = Some(1);
        for amount in [250, i32::MAX] {
            game.server.guest_equipment_breaks.clear();
            let broken_before = game.stats.get("broken", "minecraft:iron_pickaxe");
            wear.set(Rule::Set(amount));
            assert_eq!(used(game, "minecraft:iron_pickaxe", 1), u32::MAX);
            assert_eq!(wear.take_asked(), [1]);
            assert_eq!(game.server.guest_equipment_breaks, [47]);
            assert_eq!(
                game.stats.get("broken", "minecraft:iron_pickaxe"),
                broken_before + 1
            );
            game.damage_held(1);
            assert!(wear.take_asked().is_empty());
            assert_eq!(game.server.guest_equipment_breaks, [47]);
            assert_eq!(
                game.stats.get("broken", "minecraft:iron_pickaxe"),
                broken_before + 1
            );
        }
        wear.set(Rule::Set(249));
        assert_eq!(used(game, "minecraft:iron_pickaxe", 1), 249);
        assert_eq!(wear.take_asked(), [1]);
        wear.set(Rule::Set(1));
        game.damage_held(1);
        assert!(game.inventory[0].is_none());
        assert_eq!(wear.take_asked(), [1]);
    });
}

/// The part is asked with what Unbreaking leaves, not the points the use asked for: a pickaxe of
/// Unbreaking III saves a point three times in four, the part hears each wear that remains, and it
/// is never asked about a point Unbreaking saved.
#[test]
#[ignore = "daily"]
fn a_part_is_asked_after_unbreaking() {
    with_wear_part("minecraft:iron_pickaxe", Rule::Times(1), |game, wear| {
        game.world.random = sunrise_core::random::LevelRandom::for_level(
            sunrise_core::random::LegacyRandom::new(12345),
        );
        game.creative = false;
        game.selected = 0;
        let mut pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
        pickaxe.enchant("minecraft:unbreaking", 3);
        game.inventory[0] = Some(pickaxe);
        for _ in 0..200 {
            game.damage_held(1);
        }
        let asked = wear.take_asked();
        let damage = game.inventory[0].as_ref().unwrap().damage;
        // One point each, and each worn point is one the part was asked about.
        assert!(asked.iter().all(|points| *points == 1));
        assert_eq!(asked.len() as u32, damage);
        // A quarter of 200 points is 50: far from none and from all of them.
        assert!((20..=90).contains(&damage), "{damage}");

        // Two points at once reach the part as what Unbreaking left of them.
        wear.set(Rule::Halve);
        game.inventory[0].as_mut().unwrap().damage = 0;
        for _ in 0..100 {
            game.damage_held(2);
        }
        let asked = wear.take_asked();
        assert!(asked.iter().all(|points| (1..=2).contains(points)));
        let expected: i32 = asked.iter().map(|points| points / 2).sum();
        assert_eq!(game.inventory[0].as_ref().unwrap().damage as i32, expected);
    });
}

/// `break_now` breaks a pickaxe at full durability on its first wear, through the vanilla break:
/// the slot empties, the break event goes out and the stat counts it. The part is told who wears
/// the tool: the player.
#[test]
#[ignore = "daily"]
fn a_part_breaks_its_tool_now() {
    with_wear_part("minecraft:iron_pickaxe", Rule::Break, |game, wear| {
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0),
            )
            .unwrap(),
        );
        game.server.lan_acting = Some(1);
        game.server.guest_equipment_breaks.clear();
        let broken_before = game.stats.get("broken", "minecraft:iron_pickaxe");
        assert_eq!(used(game, "minecraft:iron_pickaxe", 1), u32::MAX);
        assert_eq!(wear.take_asked(), [1]);
        assert_eq!(game.server.guest_equipment_breaks, [47]);
        assert_eq!(
            game.stats.get("broken", "minecraft:iron_pickaxe"),
            broken_before + 1
        );
        let player = game
            .native_lifecycle
            .epoch()
            .map(|_| game.own_entity_id() as u32);
        assert_eq!(wear.take_worn_by(), [player]);
    });
}

/// A zombie's helmet names the zombie as the entity wearing it.
#[test]
#[ignore = "daily"]
fn a_mob_armour_wear_names_the_mob() {
    use mod_api::{EntityRef, EquipmentSlot, LivingRef};
    use sunrise_gameplay_common::game::creatures::SpawnSite;
    with_wear_part("minecraft:iron_helmet", Rule::Times(1), |game, wear| {
        let zombie = game
            .spawn_mob("minecraft:zombie", [3.5, 64.0, 6.5], true, SpawnSite::Raw)
            .unwrap();
        let helmet = Some(ItemStack::new("minecraft:iron_helmet", 1));
        let mob = game.mobs.iter_mut().find(|m| m.id == zombie).unwrap();
        mob.set_equipment_stack(EquipmentSlot::Head as usize, helmet);
        let epoch = game.native_lifecycle.epoch().unwrap();
        let living = LivingRef::new(EntityRef::new(epoch, zombie));
        game.living_hurt_and_break(living, EquipmentSlot::Head, 1);
        assert_eq!(wear.take_asked(), [1]);
        assert_eq!(wear.take_worn_by(), [Some(zombie)]);
    });
}

/// A part's `break_now` on a mob's glider breaks it through the vanilla
/// break: the slot empties instead of keeping the stack at its maximum damage.
#[test]
#[ignore = "daily"]
fn a_part_breaks_a_mobs_glider_now() {
    use sunrise_core::equipment_slot::EquipmentSlot;
    use sunrise_gameplay_common::game::creatures::SpawnSite;
    with_wear_part("minecraft:elytra", Rule::Break, |game, wear| {
        let zombie = game
            .spawn_mob("minecraft:zombie", [3.5, 64.0, 6.5], true, SpawnSite::Raw)
            .unwrap();
        let elytra = Some(ItemStack::new("minecraft:elytra", 1));
        let mob = game.mobs.iter_mut().find(|m| m.id == zombie).unwrap();
        mob.set_equipment_stack(EquipmentSlot::Chest as usize, elytra);
        game.wear_mob_glider(zombie, EquipmentSlot::Chest);
        assert_eq!(wear.take_asked(), [1]);
        assert_eq!(wear.take_worn_by(), [Some(zombie)]);
        let mob = game.mobs.iter().find(|m| m.id == zombie).unwrap();
        assert_eq!(
            mob.item_by_slot(EquipmentSlot::Chest),
            None,
            "the glider broke"
        );
    });
}

/// `ItemStack::hurt_and_break` (#5369) wears a stack nobody holds as `ItemStack.hurtAndBreak`
/// does: a point of wear, the last point breaking the stack away, nothing for an item that is
/// not damageable, Unbreaking III saving most points, and nothing for a creative player. A stale
/// level, a player handle that is not the player's and a client are refused.
#[test]
#[ignore = "daily"]
fn a_mod_wears_a_stack_nobody_holds_as_vanilla_does() {
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.creative = false;
        game.world.random = sunrise_core::random::LevelRandom::for_level(
            sunrise_core::random::LegacyRandom::new(12345),
        );
        let level = game.level(game.dim.id()).unwrap();
        let api = |stack: &ItemStack| -> mod_api::ItemStack { stack.try_into_api().unwrap() };
        let pickaxe = api(&ItemStack::new("minecraft:iron_pickaxe", 1));
        let worn = pickaxe
            .hurt_and_break(game, level, 1, None)
            .unwrap()
            .unwrap();
        assert_eq!(worn.damage(), 1);
        // An iron pickaxe holds 250 points: its last one breaks it away.
        let mut nearly = ItemStack::new("minecraft:iron_pickaxe", 1);
        nearly.damage = 249;
        assert_eq!(api(&nearly).hurt_and_break(game, level, 1, None), Ok(None));
        let stone = api(&ItemStack::new("minecraft:stone", 4));
        assert_eq!(
            stone.hurt_and_break(game, level, 1, None),
            Ok(Some(stone.clone()))
        );
        let mut tough = ItemStack::new("minecraft:iron_pickaxe", 1);
        tough.enchant("minecraft:unbreaking", 3);
        let mut tough = api(&tough);
        for _ in 0..200 {
            tough = tough.hurt_and_break(game, level, 1, None).unwrap().unwrap();
        }
        // A quarter of 200 points is 50: far from none and from all of them.
        assert!((20..=90).contains(&tough.damage()), "{}", tough.damage());

        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0),
            )
            .unwrap(),
        );
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
        let player = game.player(123).unwrap();
        game.creative = true;
        assert_eq!(
            pickaxe.hurt_and_break(game, level, 1, Some(player)),
            Ok(Some(pickaxe.clone()))
        );
        game.creative = false;
        // Refusals: a stale level, a player handle whose uuid is not the player's, a client.
        let stale = mod_api::LevelRef::new(mod_api::WorldEpoch::new(u64::MAX), game.dim.key());
        assert_eq!(
            pickaxe.hurt_and_break(game, stale, 1, None),
            Err(mod_api::StackWriteError::Unavailable)
        );
        let impostor = mod_api::__private::game::player_ref(game.entity(20).unwrap(), 999);
        assert_eq!(
            pickaxe.hurt_and_break(game, level, 1, Some(impostor)),
            Err(mod_api::StackWriteError::Unavailable)
        );
        let worn = pickaxe
            .hurt_and_break(game, level, 1, Some(player))
            .unwrap()
            .unwrap();
        assert_eq!(worn.damage(), 1);

        // A client wears nothing.
        game.options.dedicated = None;
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        assert_eq!(
            pickaxe.hurt_and_break(game, level, 1, None),
            Err(mod_api::StackWriteError::Unavailable)
        );
    });
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
