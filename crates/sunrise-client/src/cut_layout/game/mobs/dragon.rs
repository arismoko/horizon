#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay_common::game::creatures::SpawnSite;
use sunrise_gameplay_common::game::dragon_common::Dragon;
use sunrise_sim::mobs::DamageKind;
use sunrise_sim::projectiles::ProjectileKind;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::server_side::dragon::{DragonHitHooks, DragonHitOrigin, Hurt};
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::dragon_common::Phase;
    use sunrise_gameplay_common::game::server_side::dragon::tests::dragon;
    use sunrise_gameplay_common::game::server_side::dragon::tests::flying;
    use sunrise_protocol::entity_data::Value;

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn a_spawn_egg_dragon_joins_once_and_a_canceled_spawn_fails() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks, ModEntry, ServerHost};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        struct Observer {
            joins: Arc<Mutex<Vec<(u32, bool)>>>,
            cancel: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind != "minecraft:ender_dragon" {
                    return Flow::Continue;
                }
                self.joins
                    .lock()
                    .unwrap()
                    .push((event.subject.id(), event.loaded_from_disk));
                if self.cancel.load(Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let joins = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "dragon-joins",
            Box::new(Observer {
                joins: joins.clone(),
                cancel: cancel.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.dragon.discard();
                game.native_joins_skip();
                let at = [4.5, 200.0, 4.5];
                let id = game
                    .spawn_mob_spawned(
                        "minecraft:ender_dragon",
                        at,
                        sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                            sunrise_sim::mobs::spawn_rules::Reason::SpawnItemUse,
                        ),
                    )
                    .unwrap();
                assert_eq!(game.dragon.as_ref().unwrap().id, id);
                assert_eq!(*joins.lock().unwrap(), vec![(id, false)]);
                game.native_joins_flush(false, 0);
                assert_eq!(joins.lock().unwrap().len(), 1, "a join is told only once");
                let dispenser =
                    SpawnSite::Finalizing(sunrise_sim::mobs::spawn_rules::Reason::Dispenser);
                assert!(game.spawn_dragon_spawned(at, dispenser).is_none());
                assert_eq!(joins.lock().unwrap().len(), 1, "no second dragon joins");

                game.dragon.discard();
                cancel.store(true, Ordering::Relaxed);
                // The dispenser calls the shared constructor directly.
                assert!(game.spawn_dragon_spawned(at, dispenser).is_none());
                assert!(game.dragon.is_none(), "a refused dragon never stays loaded");
                game.native_joins_flush(false, 0);
                assert_eq!(*joins.lock().unwrap(), vec![(id, false), (id + 9, false)]);
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn dragon_finalization_preserves_the_site_and_both_cancels() {
        use mod_api::{FinalizeSpawnEvent, Flow, Hooks, Host, ModEntry};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_sim::mobs::spawn_rules::Reason;

        #[derive(Clone, Copy, PartialEq)]
        enum Answer {
            Keep,
            Skip,
            Refuse,
        }
        struct Listener {
            answer: Answer,
            reason: Reason,
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
                assert_eq!(event.kind, "minecraft:ender_dragon");
                assert_eq!(event.reason, self.reason);
                assert!(event.spawner.is_none());
                self.calls.fetch_add(1, Ordering::Relaxed);
                match self.answer {
                    Answer::Keep => Flow::Continue,
                    Answer::Skip => Flow::Cancel,
                    Answer::Refuse => {
                        event.spawn_cancelled = true;
                        Flow::Continue
                    }
                }
            }
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            for reason in [Reason::SpawnItemUse, Reason::Dispenser] {
                for answer in [Answer::Keep, Answer::Skip, Answer::Refuse] {
                    game.dragon.discard();
                    game.world.random.set_seed(42);
                    let calls = Arc::new(AtomicUsize::new(0));
                    let mods: mod_api::Mods = vec![ModEntry::new(
                        "dragon-finalize",
                        Box::new(Listener {
                            answer,
                            reason,
                            calls: calls.clone(),
                        }),
                    )]
                    .into();
                    let result = mod_platform::testing::with_mods(mods, || {
                        game.spawn_dragon_spawned([4.5, 200.0, 4.5], SpawnSite::Finalizing(reason))
                    });
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert_eq!(result.is_none(), answer == Answer::Refuse);
                    assert_eq!(game.dragon.is_none(), answer == Answer::Refuse);
                    // snapTo always draws yaw; only completed Mob.finalizeSpawn
                    // draws the follow-range triangle and handedness roll.
                    let mut expected = LegacyRandom::new(42);
                    let yaw =
                        sunrise_level::camera_math::wrap_degrees(expected.next_float() * 360.0);
                    if let Some(dragon) = game.dragon.as_ref() {
                        assert_eq!(dragon.yaw, yaw);
                        assert_eq!(dragon.previous_yaw, yaw);
                    }
                    if answer == Answer::Keep {
                        expected.triangle(0.0, 0.114_850_000_000_000_01);
                        expected.next_float();
                    }
                    assert_eq!(game.world.random.next_long(), expected.next_long());
                }
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn dragon_summon_command_finalizes_only_without_nbt() {
        use mod_api::{
            EntityJoinLevelEvent, FinalizeSpawnEvent, Flow, Hooks, Host, ModEntry, ServerHost,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_sim::mobs::spawn_rules::Reason;

        #[derive(Clone, Copy, PartialEq)]
        enum Answer {
            Keep,
            Skip,
            Refuse,
            RefuseJoin,
        }
        struct Listener {
            answer: Answer,
            reason: Reason,
            calls: Arc<AtomicUsize>,
            joins: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                assert_eq!(event.kind, "minecraft:ender_dragon");
                assert!(!event.loaded_from_disk);
                self.joins.fetch_add(1, Ordering::Relaxed);
                if self.answer == Answer::RefuseJoin {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
                assert_eq!(event.kind, "minecraft:ender_dragon");
                assert_eq!(event.reason, self.reason);
                assert!(event.spawner.is_none());
                self.calls.fetch_add(1, Ordering::Relaxed);
                match self.answer {
                    Answer::Keep | Answer::RefuseJoin => Flow::Continue,
                    Answer::Skip => Flow::Cancel,
                    Answer::Refuse => {
                        event.spawn_cancelled = true;
                        Flow::Continue
                    }
                }
            }
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            for nbt in [None, Some("{}"), Some("{Silent:1b}")] {
                let reason = Reason::Command;
                for answer in [
                    Answer::Keep,
                    Answer::Skip,
                    Answer::Refuse,
                    Answer::RefuseJoin,
                ] {
                    game.dragon.discard();
                    game.world.random.set_seed(42);
                    let calls = Arc::new(AtomicUsize::new(0));
                    let joins = Arc::new(AtomicUsize::new(0));
                    let mods: mod_api::Mods = vec![ModEntry::new(
                        "dragon-finalize",
                        Box::new(Listener {
                            answer,
                            reason,
                            calls: calls.clone(),
                            joins: joins.clone(),
                        }),
                    )]
                    .into();
                    let result = mod_platform::testing::with_mods(mods, || {
                        game.build_registries();
                        game.set_allow_commands(true);
                        let suffix = nbt.map(|tag| format!(" {tag}")).unwrap_or_default();
                        game.run_command(&format!(
                            "summon minecraft:ender_dragon 4.5 200 4.5{suffix}"
                        ))
                    });
                    assert_eq!(calls.load(Ordering::Relaxed), usize::from(nbt.is_none()));
                    let refused_finalization = nbt.is_none() && answer == Answer::Refuse;
                    assert_eq!(
                        joins.load(Ordering::Relaxed),
                        usize::from(!refused_finalization)
                    );
                    let refused = refused_finalization || answer == Answer::RefuseJoin;
                    assert_eq!(result, !refused);
                    assert_eq!(game.dragon.is_none(), refused);
                    if !refused {
                        assert_eq!(game.dragon_silent(), nbt == Some("{Silent:1b}"));
                        let dragon = game.dragon.as_ref().unwrap();
                        assert_eq!(dragon.position, Vec3::new(4.5, 200.0, 4.5));
                        assert_eq!(dragon.yaw, 0.0);
                        assert_eq!(dragon.previous_yaw, 0.0);
                    }
                    // Command construction consumes no placement yaw. Mob.finalizeSpawn
                    // draws follow range and handedness unless skipped. The NBT
                    // loader seeds the dragon's own random without drawing from the level RNG.
                    let mut expected = LegacyRandom::new(42);
                    if nbt.is_none() && matches!(answer, Answer::Keep | Answer::RefuseJoin) {
                        expected.triangle(0.0, 0.114_850_000_000_000_01);
                        expected.next_float();
                    }
                    assert_eq!(game.world.random.next_long(), expected.next_long());
                }
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn carry_dragon_dispatches_only_after_phase_and_source_guards() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Probe(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Probe {
            fn entity_invulnerability_check(
                &self,
                _: mod_api::LevelSide<'_>,
                _: &mut mod_api::EntityInvulnerabilityCheckEvent<'_>,
            ) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let heard = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.dragon.replace(dragon(Phase::Scanning { time: 0 }));
                let arrow = ProjectileKind::Trident;
                let hit = |game: &mut sunrise_gameplay::game::ServerGame<'_>, counts, direct| {
                    game.hurt_dragon_part_at(
                        0,
                        2.0,
                        counts,
                        None,
                        DamageKind::Generic,
                        DragonHitOrigin {
                            direct,
                            entity_id: None,
                            from: None,
                            invulnerability_checked: false,
                            hook_source: None,
                            attack: None,
                            hooks: DragonHitHooks::capture(),
                        },
                    )
                };
                assert_eq!(hit(game, true, Some(&arrow)), Hurt::Shrugged);
                assert_eq!(heard.load(Ordering::SeqCst), 0);
                game.dragon.replace(dragon(flying()));
                let id = game.dragon.as_ref().unwrap().id;
                assert!(game.command_damage_dragon(
                    id,
                    2.0,
                    "minecraft:generic",
                    DamageKind::Generic,
                    (None, None, None)
                ));
                assert_eq!(heard.load(Ordering::SeqCst), 0);
                assert!(matches!(hit(game, false, None), Hurt::Landed { .. }));
                assert_eq!(heard.load(Ordering::SeqCst), 0);
                hit(game, true, None);
                assert_eq!(heard.load(Ordering::SeqCst), 1);
            });
        });
    }
    /// `dealDefaultKnockback` reads the dragon's `getAttributeValue(KNOCKBACK_RESISTANCE)` at the
    /// hit: a netherite chestplate's `0.1F` (`ArmorMaterials.NETHERITE`, widened to the
    /// modifier's double) shortens the push to `0.4F * (1 - 0.1F)`.
    #[test]
    #[ignore = "daily"]
    fn a_hit_on_the_dragon_pushes_by_its_live_knockback_resistance() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut d = dragon(flying());
            d.velocity = Vec3::new(1.0, -0.3, 0.5);
            d.equipment.set(
                sunrise_items::equipment::CHEST,
                Some(sunrise_items::items::ItemStack::new(
                    "minecraft:netherite_chestplate",
                    1,
                )),
            );
            game.dragon.replace(d);
            game.refresh_stand_and_dragon_equipment_attributes();
            let resistance = f64::from(0.1_f32);
            assert_eq!(game.dragon_knockback_resistance(), resistance);
            let from = Vec3::new(3.0, 70.0, 4.0);
            let hurt = game.hurt_dragon_part_at(
                0,
                9.0,
                true,
                None,
                DamageKind::Generic,
                DragonHitOrigin {
                    direct: None,
                    entity_id: None,
                    from: Some(from),
                    invulnerability_checked: false,
                    hook_source: None,
                    attack: None,
                    hooks: DragonHitHooks::capture(),
                },
            );
            assert_eq!(hurt, Hurt::Landed { full: true });
            let power = f64::from(0.4_f32) * (1.0 - resistance);
            assert_eq!(
                game.dragon.as_ref().unwrap().velocity,
                Vec3::new(0.5 - 0.6 * power, -0.3, 0.25 - 0.8 * power)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_stationary_no_ai_dragon_runs_its_living_body_turn() {
        use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
        sunrise_gameplay::game::native_entity_refs::tests::with_resident_game(|game| {
            game.create_new_dragon();
            let dragon = game.common.dragon.as_mut().unwrap();
            assert_ne!(dragon.yaw, 0.0);
            assert_eq!((dragon.body_yaw, dragon.head_yaw), (0.0, 0.0));
            dragon.yaw = 35.0;
            dragon.no_ai = true;
            dragon.head_yaw = 90.0;
            let id = dragon.id;
            let entity = game.entity(id).unwrap();
            assert_eq!(entity.set_body_yaw(game, 0.0), Ok(true));
            game.tick_dragon();
            // A 75-degree limit turns the body 15 degrees toward the head.
            assert_eq!(entity.body_yaw(game), Some(15.0));
            assert_eq!(game.common.dragon.as_ref().unwrap().yaw, 35.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_tracked_dragon_lists_its_data_defaults() {
        use sunrise_protocol::entity_data::serializer;
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon.replace(dragon);
            let state = game.lan_dragon_state().unwrap();
            assert_eq!(
                state.defaults,
                vec![
                    (9, serializer::FLOAT, Value::Float(1.0)),
                    (15, serializer::BYTE, Value::Byte(0)),
                    (16, serializer::INT, Value::Int(10)),
                ]
            );
        });
    }
}

#[cfg(test)]
#[path = "dragon/residency_tests.rs"]
pub(crate) mod residency_tests;
#[cfg(test)]
#[path = "dragon/thorns_tests.rs"]
pub(crate) mod thorns_tests;
#[cfg(test)]
#[path = "dragon/walls_tests.rs"]
pub(crate) mod walls_tests;

#[cfg(test)]
pub(crate) mod dragon_fireball_spawn_tests {
    use sunrise_gameplay_common::game::GameGameExt1;
    include!("dragon/fireball_spawn_tests.rs");
}
#[cfg(test)]
#[path = "dragon/wing_push_tests.rs"]
pub(crate) mod wing_push_tests;
