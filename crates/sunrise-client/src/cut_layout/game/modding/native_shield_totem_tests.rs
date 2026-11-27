//! `living_shield_block` and `living_use_totem` heard by a trait mod through the game's own
//! hooks and the hit's level host: when each fires in a hit, and what a mod may change.
use super::native_entity_refs::tests::with_game;
use mod_api::{
    self, Hand, HitHost, Hooks, LivingDamagePostEvent, LivingDeathProtectionEvent,
    LivingShieldBlockEvent, LivingUseTotemEvent,
};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::{Damage, DamageKind, Mob};

#[derive(Default)]
struct Heard {
    /// Each hook in the order the hit fired it.
    order: Vec<&'static str>,
    /// The shield hook's entity, damage and whether vanilla blocked.
    shield: Vec<(i32, f32, bool)>,
    totem: Vec<(i32, Hand, String)>,
    /// What the shield hook sets: `blocked`, `blocked_damage`.
    set_block: Option<(bool, f32)>,
    refuse_hand: Option<Hand>,
    random_draw: bool,
    /// What `living_death_protection` answers with, if anything: a worn accessory's item, one.
    lend: Option<&'static str>,
}

struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_shield_block(&self, level: &mut dyn HitHost, event: &mut LivingShieldBlockEvent<'_>) {
        let mut heard = self.0.lock().unwrap();
        heard.order.push("shield");
        heard
            .shield
            .push((event.entity, event.damage, event.original_blocked));
        if let Some((blocked, damage)) = heard.set_block {
            event.block.blocked = blocked;
            event.block.blocked_damage = damage;
        }
        if heard.random_draw {
            level.with_random(|random| random.next_int());
        }
    }

    fn living_death_protection(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingDeathProtectionEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard.order.push("protection");
        if event.protector.is_none()
            && let Some(item) = heard.lend.take()
        {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let stack = sunrise_items::items::ItemStack::new(item, 1);
            *event.protector = Some(stack.try_into_api().unwrap());
        }
    }
    fn living_damage_post(&self, _: &mut dyn HitHost, _: &mut LivingDamagePostEvent<'_>) {
        self.0.lock().unwrap().order.push("post");
    }
    fn living_use_totem(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingUseTotemEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.order.push("totem");
        heard.totem.push((
            event.entity,
            event.hand,
            mod_api::__private::item_key_id(event.stack.item())
                .unwrap()
                .to_owned(),
        ));
        if heard.refuse_hand == Some(event.hand) {
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
}

fn heard(setup: Heard, test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) -> Heard {
    let log = Arc::new(Mutex::new(setup));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(test));
    std::mem::take(&mut *log.lock().unwrap())
}

fn zombie(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Mob {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    Mob::new(id, kind, Vec3::ZERO, 0.0)
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn the_shield_hook_sees_the_hit_before_armor_and_enchantment_reductions() {
    let heard = heard(Heard::default(), |game| {
        let mut hit = zombie(game, 226001);
        // A mob's attack: its armor takes a share of the 10 later, in `actuallyHurt`.
        assert!(hit.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            10.0,
            Damage::mob(226002, [2.0, 0.0, 2.0])
        ));
        assert!(hit.health < 20.0);
    });
    assert_eq!(heard.shield, [(226001, 10.0, false)]);
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn a_blocked_damage_above_the_hit_is_clamped_to_it() {
    let setup = Heard {
        set_block: Some((true, 100.0)),
        ..Heard::default()
    };
    let heard = heard(setup, |game| {
        let mut hit = zombie(game, 226003);
        let before = hit.health;
        hit.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            6.0,
            Damage::of(DamageKind::Generic),
        );
        // All 6 are stopped, no more: the zombie lost nothing.
        assert_eq!(hit.health, before);
    });
    assert_eq!(heard.shield, [(226003, 6.0, false)]);
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn setting_blocked_false_blocks_nothing() {
    let setup = Heard {
        set_block: Some((false, 6.0)),
        ..Heard::default()
    };
    let heard = heard(setup, |game| {
        let mut hit = zombie(game, 226004);
        let before = hit.health;
        hit.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            6.0,
            Damage::of(DamageKind::Generic),
        );
        assert!(hit.health < before);
    });
    assert_eq!(heard.shield.len(), 1);
}

#[test]
#[ignore = "daily"]
fn the_totem_hook_fires_after_the_health_is_gone_and_a_cancel_lets_the_mob_die() {
    let setup = Heard {
        refuse_hand: Some(Hand::Main),
        ..Heard::default()
    };
    let heard = heard(setup, |game| {
        let mut hit = zombie(game, 226005);
        hit.extra
            .set_main_hand(Some("minecraft:totem_of_undying".into()));
        game.mobs.push(hit);
        assert!(game.mob_hits_mob(226005, 1000.0, Damage::of(DamageKind::Generic)));
        assert_eq!(game.mobs[0].health, 0.0);
    });
    assert_eq!(
        heard.totem,
        [(226005, Hand::Main, "minecraft:totem_of_undying".to_string())]
    );
    assert_eq!(heard.order, ["shield", "totem", "protection", "post"]);
}

#[test]
#[ignore = "daily"]
fn an_uncanceled_totem_saves_the_mob() {
    use sunrise_core::nbt::Tag;
    let heard = heard(
        Heard {
            refuse_hand: Some(Hand::Main),
            ..Heard::default()
        },
        |game| {
            let mut hit = zombie(game, 226006);
            let main = sunrise_items::items::ItemStack::new("minecraft:totem_of_undying", 1);
            let mut off = sunrise_items::items::ItemStack::new("minecraft:stick", 2);
            off.data_mut().set_other(
                "minecraft:death_protection",
                Some(Tag::compound([] as [(&str, Tag); 0])),
            );
            hit.extra.set_main_hand_stack(Some(main.clone()));
            hit.extra
                .set_slot_stack(sunrise_items::equipment::OFFHAND, Some(off));
            game.mobs.push(hit);
            assert!(game.mob_hits_mob(226006, 1000.0, Damage::of(DamageKind::Generic)));
            let hit = &game.mobs[0];
            assert_eq!(hit.health, 1.0);
            assert_eq!(hit.extra.main_hand_stack(), Some(&main));
            assert_eq!(
                hit.extra
                    .slot_stack(sunrise_items::equipment::OFFHAND)
                    .unwrap()
                    .count,
                1
            );
        },
    );
    assert_eq!(
        heard.totem,
        [
            (226006, Hand::Main, "minecraft:totem_of_undying".to_owned()),
            (226006, Hand::Off, "minecraft:stick".to_owned()),
        ]
    );
    assert_eq!(heard.order, ["shield", "totem", "totem", "post"]);
}

#[test]
#[ignore = "daily"]
fn a_mods_protector_from_outside_the_hands_saves_the_mob() {
    let totem = sunrise_items::items::ItemStack::new("minecraft:totem_of_undying", 1);
    let saved = heard(
        Heard {
            refuse_hand: Some(Hand::Main),
            lend: Some("minecraft:totem_of_undying"),
            ..Heard::default()
        },
        |game| {
            let mut hit = zombie(game, 226008);
            hit.extra.set_main_hand_stack(Some(totem.clone()));
            game.mobs.push(hit);
            assert!(game.mob_hits_mob(226008, 1000.0, Damage::of(DamageKind::Generic)));
            let hit = &game.mobs[0];
            assert_eq!(hit.health, 1.0, "saved");
            assert_eq!(
                hit.extra.main_hand_stack(),
                Some(&totem),
                "the refused hand keeps its totem"
            );
        },
    );
    assert_eq!(saved.order, ["shield", "totem", "protection", "post"]);
    // With no answer the mob dies, and a stack without the component is no answer.
    let died = heard(
        Heard {
            lend: Some("minecraft:stick"),
            ..Heard::default()
        },
        |game| {
            let attacker = zombie(game, 226009);
            game.mobs.push(attacker);
            assert!(game.mob_hits_mob(226009, 1000.0, Damage::of(DamageKind::Generic)));
            assert_eq!(game.mobs[0].health, 0.0);
        },
    );
    assert_eq!(died.order, ["shield", "protection", "post"]);
}

/// The hit's level is the one the hit draws from: a mod's draw moves the world's random on by
/// exactly one place.
#[cfg(any())]
#[test]
#[ignore = "daily"]
fn the_shield_hook_draws_from_the_hits_random() {
    let after = |random_draw| {
        let next = Mutex::new(0);
        heard(
            Heard {
                random_draw,
                ..Heard::default()
            },
            |game| {
                *game.world.random = sunrise_core::random::LegacyRandom::new(5);
                let mut hit = zombie(game, 226007);
                hit.damage(
                    damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
                    1.0,
                    Damage::of(DamageKind::Generic),
                );
                *next.lock().unwrap() = game.world.random.next_int();
            },
        );
        next.into_inner().unwrap()
    };
    // The world's random is one stream: the mod's draw puts the value after the hit one
    // place further on than it is without the mod.
    let without = after(false);
    let mut stream = sunrise_core::random::LegacyRandom::new(5);
    let place = (0..64).find(|_| stream.next_int() == without).unwrap();
    assert_eq!(after(true), stream.next_int(), "after {place} draws");
}
