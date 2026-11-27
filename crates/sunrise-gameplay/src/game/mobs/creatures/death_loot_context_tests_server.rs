use super::*;
use std::sync::{Arc, Mutex};

pub(in crate::game) type Observation = (String, i32, i32, Option<u32>);
pub(in crate::game) struct Probe(pub(in crate::game) Arc<Mutex<Vec<Observation>>>);
// Veto the first death; the next victim must still reach the drop hook.
pub(in crate::game) struct CancelOnce(pub(in crate::game) std::sync::atomic::AtomicBool);
#[mod_api::__subscriptions]
impl mod_api::Hooks for CancelOnce {
    fn living_death(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        if self.0.swap(false, std::sync::atomic::Ordering::Relaxed) {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Probe {
    fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_, '_>,
    ) {
        let source = event.context.damage_source.unwrap();
        assert!(event.context.tool.is_none(), "entity context has no TOOL");
        let hand = host
            .entity(source.causing as u32)
            .and_then(|entity| host.living(entity))
            .and_then(|living| {
                living.with_hand_item(host, mod_api::Hand::Main, |stack| {
                    (stack.damage(), stack.count())
                })
            });
        if source.causing == 12 {
            assert_eq!(
                hand.map(|(damage, _)| damage),
                Some(31),
                "later listeners read the earlier loot listener's hand edit"
            );
        }
        self.0.lock().unwrap().push((
            source.damage_type.into(),
            source.causing,
            source.direct,
            hand.map(|(_, count)| count),
        ));
    }
}

pub(in crate::game) struct ChangeKillerHand(pub(in crate::game) mod_api::ItemStack);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ChangeKillerHand {
    fn living_death(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        let entity = host.entity(12).unwrap();
        let living = host.living(entity).unwrap();
        assert!(
            living
                .set_hand_item(host, mod_api::Hand::Main, Some(self.0.clone()))
                .unwrap()
        );
        mod_api::Flow::Continue
    }
    fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_, '_>,
    ) {
        assert!(event.context.tool.is_none());
        let entity = host
            .entity(event.context.damage_source.unwrap().causing as u32)
            .unwrap();
        let living = host.living(entity).unwrap();
        assert_eq!(
            living
                .with_hand_item(host, mod_api::Hand::Main, |stack| stack.project())
                .flatten(),
            Some(self.0.clone()),
            "loot sees the complete stack installed during living_death"
        );
        assert!(
            living
                .set_hand_item(
                    host,
                    mod_api::Hand::Main,
                    Some({
                        let mut later = self.0.clone();
                        later.set_damage(31);
                        later
                    })
                )
                .unwrap()
        );
    }
}

pub(in crate::game) struct Nested;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Nested {
    fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_, '_>,
    ) {
        if event.context.entity.map(|e| e.0) != Some(90) {
            return;
        }
        let source = event.context.damage_source;
        let tool = event.context.tool.cloned();
        assert!(host.commands().run_command("kill @e[type=minecraft:pig]"));
        assert_eq!(event.context.damage_source, source);
        assert_eq!(event.context.tool, tool.as_ref());
    }
}

#[test]
#[ignore = "daily"]
fn death_loot_killer_is_not_the_acting_guest() {
    use crate::game::native_guest_turn_tests::with_guests;
    use sunrise_sim::mobs::Damage;
    with_guests(|game| {
        game.server.next_entity = 1000;
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server.server_loot.0.set(Some(Arc::new(loot)));
        game.build_registries();
        let mut hand = ItemStack::new("minecraft:diamond_sword", 1);
        hand.damage = 19;
        hand.enchant("minecraft:looting", 2);
        let mut changed = hand.clone();
        changed.damage = 27;
        let expected = changed.try_into_api().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            mod_api::ModEntry::new("change_hand", Box::new(ChangeKillerHand(expected))),
            mod_api::ModEntry::new("guest_death_loot_probe", Box::new(Probe(seen.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let ty = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            for (id, acting) in [(80, Some(13)), (81, None)] {
                game.as_guest(12, |g| {
                    g.common.inventory[g.common.selected] = Some(hand.clone());
                })
                .unwrap();
                let mut victim = Mob::new(id, ty.clone(), Vec3::ZERO, 0.0);
                victim.ai.last_hurt_by_player = 100;
                victim.last_damage = Some((Damage::player([0.0; 3]), 0));
                game.server.last_hurt_by_player.insert(
                    id,
                    sunrise_gameplay_common::game::combat_common::PlayerCredit(12),
                );
                if let Some(conn) = acting {
                    game.as_guest(conn, |g| g.mob_die(&victim)).unwrap();
                } else {
                    assert!(game.server.lan_acting.is_none());
                    game.mob_die(&victim);
                }
            }
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [
                ("minecraft:player_attack".into(), 12, 12, Some(1)),
                ("minecraft:player_attack".into(), 12, 12, Some(1))
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn death_loot_nested_death_keeps_outer_context() {
    use crate::game::native_guest_turn_tests::with_guests as with_game;
    use sunrise_sim::mobs::{Damage, DamageKind, Who};
    with_game(|game| {
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server.server_loot.0.set(Some(Arc::new(loot)));
        let ty = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        assert!(game.run_command("summon pig 5.5 64 8.5 {NoAI:1b,NoGravity:1b}"));

        let mut killer = Mob::new(41, ty.clone(), Vec3::ZERO, 0.0);
        killer
            .extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:diamond_sword", 7)));
        game.mobs.push(killer);
        let mut victim = Mob::new(90, ty, Vec3::ZERO, 0.0);
        let mut damage = Damage::of(DamageKind::MobAttack);
        damage.attacker = Some(Who::Mob(41));
        victim.last_damage = Some((damage, 0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            mod_api::ModEntry::new("nested", Box::new(Nested)),
            mod_api::ModEntry::new("nested_probe", Box::new(Probe(seen.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || game.mob_die(&victim));
        assert_eq!(
            *seen.lock().unwrap(),
            [
                ("minecraft:generic_kill".into(), 0, 0, None),
                ("minecraft:mob_attack".into(), 41, 41, Some(7))
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn death_loot_uses_call_scoped_source_and_hand() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_resident_game};
    use sunrise_sim::mobs::{Damage, DamageKind, Who};
    if run_fresh_cases(
        "game::server_side::creatures_server::death_loot_context_tests::death_loot_uses_call_scoped_source_and_hand",
        "SUNRISE_DEATH_LOOT_CONTEXT_CHILD",
        &["damage_sources"],
    ) {
        return;
    }
    with_resident_game(|game| {
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server.server_loot.0.set(Some(Arc::new(loot)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "death_loot_probe",
            Box::new(Probe(seen.clone())) as Box<dyn mod_api::Hooks>,
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let ty = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 11, 123);
            // Reserve the fixture's explicit IDs before spawning experience orbs.
            game.server.next_entity = 1000;
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:iron_sword", 2));
            for (id, environmental) in [(68, false), (69, true)] {
                let mut victim = Mob::new(id, ty.clone(), Vec3::ZERO, 0.0);
                victim.ai.last_hurt_by_player = 100;
                game.remember_mob_player_credit(id);
                victim.last_damage = Some((
                    if environmental {
                        Damage::fall()
                    } else {
                        Damage::player([0.0; 3])
                    },
                    0,
                ));
                game.mob_die(&victim);
            }
            let mut killer = Mob::new(41, ty.clone(), Vec3::ZERO, 0.0);
            killer
                .extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:diamond_sword", 7)));
            game.mobs.push(killer);
            for (id, kind, attacker) in [
                (70, DamageKind::MobAttack, Some(Who::Mob(41))),
                (71, DamageKind::Arrow, Some(Who::Mob(41))),
                (72, DamageKind::Fall, None),
            ] {
                let mut victim = Mob::new(id, ty.clone(), Vec3::ZERO, 0.0);
                let mut damage = Damage::of(kind);
                damage.attacker = attacker;
                victim.last_damage = Some((damage, 0));
                game.mob_die(&victim);
            }
            // Predicate attempts cannot become the source of a later unrelated death.
            for (id, refused) in [(76, false), (77, true)] {
                let mut victim = Mob::new(id, ty.clone(), Vec3::ZERO, 0.0);
                victim.invulnerable_flag = refused;
                victim.last_damage = Some((Damage::fall(), 0));
                game.mobs.push(victim);
                let landed = game.mob_hits_mob(
                    id,
                    if refused { 5.0 } else { 0.0 },
                    Damage::player([0.0; 3]),
                );
                if refused {
                    assert!(!landed);
                }
                let victim = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
                victim.invulnerable_flag = false;
                victim.invulnerable = 0;
                let before = seen.lock().unwrap().len();
                assert!(game.mob_hits_mob(id, 1000.0, Damage::fall()));
                assert_eq!(
                    seen.lock().unwrap()[before..],
                    [("minecraft:fall".to_owned(), 0, 0, None)],
                );
            }
            // IDs belong to the killing blow, not a lookup of a surviving mob.
            game.mobs.retain(|m| m.id != 41);
            let mut victim = Mob::new(73, ty.clone(), Vec3::ZERO, 0.0);
            let mut damage = Damage::of(DamageKind::Arrow);
            damage.attacker = Some(Who::Mob(41));
            victim.last_damage = Some((damage, 0));
            game.mob_die(&victim);
            let mods = vec![
                mod_api::ModEntry::new(
                    "cancel_once",
                    Box::new(CancelOnce(std::sync::atomic::AtomicBool::new(true)))
                        as Box<dyn mod_api::Hooks>,
                ),
                mod_api::ModEntry::new(
                    "cancellation_probe",
                    Box::new(Probe(seen.clone())) as Box<dyn mod_api::Hooks>,
                ),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                let before = seen.lock().unwrap().len();
                game.mobs.push(Mob::new(78, ty.clone(), Vec3::ZERO, 0.0));
                assert!(game.mob_hits_mob(78, 1000.0, Damage::fall()));
                assert_eq!(seen.lock().unwrap().len(), before);
                game.mobs.push(Mob::new(79, ty, Vec3::ZERO, 0.0));
                assert!(game.mob_hits_mob(79, 1000.0, Damage::fall()));
                assert_eq!(seen.lock().unwrap().len(), before + 1);
            });
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [
                ("minecraft:player_attack".into(), 11, 11, Some(2)),
                ("minecraft:fall".into(), 0, 0, None),
                ("minecraft:mob_attack".into(), 41, 41, Some(7)),
                ("minecraft:arrow".into(), 41, 0, Some(7)),
                ("minecraft:fall".into(), 0, 0, None),
                ("minecraft:fall".into(), 0, 0, None),
                ("minecraft:fall".into(), 0, 0, None),
                ("minecraft:arrow".into(), 41, 0, None),
                ("minecraft:fall".into(), 0, 0, None),
            ]
        );
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

pub(in crate::game) struct BlockTool(pub(in crate::game) mod_api::ItemStack);
#[mod_api::__subscriptions]
impl mod_api::Hooks for BlockTool {
    fn loot_drops(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_, '_>,
    ) {
        assert!(event.context.damage_source.is_none());
        assert!(event.context.block.is_some());
        assert_eq!(event.context.tool, Some(&self.0));
    }
}
#[test]
#[ignore = "daily"]
fn death_loot_change_preserves_block_tool() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        let mut tool = ItemStack::new("minecraft:diamond_pickaxe", 1);
        tool.damage = 17;
        tool.enchant("minecraft:fortune", 3);
        let expected = tool.try_into_api().unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let mods = vec![mod_api::ModEntry::new(
            "block_tool",
            Box::new(BlockTool(expected)),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            game.native_block_loot([0, 64, 0], stone, None, Some(&tool), None, &mut Vec::new());
        });
    });
}
