#![cfg(test)]
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Direction;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::server_side::potions_server::PotionOwner;
use sunrise_items::items::ItemStack;
use sunrise_level::brewing;
use sunrise_level::effects::EffectInstance;
use sunrise_level::effects::PlayerEffects;
use sunrise_sim::mobs::Mob;
use sunrise_sim::projectiles::Projectile;
use sunrise_sim::projectiles::ProjectileKind;
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sunrise_gameplay::game::server_side::potions_server::collect_dragon_breath;
    use sunrise_gameplay::game::server_side::potions_server::potion_cloud_to_nbt;
    use sunrise_gameplay_common::game::client_side::potions::tests::FoodEffectMod;
    use sunrise_gameplay_common::game::client_side::potions::tests::OtherEffectMod;
    use sunrise_sim::motion::PlayerMotion;

    pub(crate) fn effect_test_mod(
        extra: impl IntoIterator<Item = mod_api::ModEntry>,
    ) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        use mod_api::{self, *};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering::Relaxed},
        };

        struct TestMod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let key = r.effect_key("test:effect");
                r.mob_effect_part(&key, TestMod(self.0.clone())).unwrap();
            }
        }
        #[mod_api::sunrise_part]
        impl MobEffectPart for TestMod {
            fn should_apply_effect_tick_this_tick(
                &self,
                _: &dyn Host,
                event: &mut ShouldApplyEffectTickThisTickEvent,
            ) {
                event.apply = true;
            }
            fn apply_effect_tick(&self, _: &dyn Host, event: &mut ApplyEffectTickEvent<'_>) {
                self.0.fetch_add(1, Relaxed);
                match event.amplifier {
                    0 => {
                        event.heal(10.0);
                        event.eat(4, 0.5);
                        event.add_exhaustion(1.5);
                    }
                    1 => event.hurt("minecraft:magic", 3.0),
                    3 if event.health > 1.0 => event.hurt("minecraft:magic", 1.0),
                    3 => {}
                    4 => {
                        event.hurt("minecraft:wither", 1.0);
                        event.hurt("minecraft:indirect_magic", 2.0);
                        event.heal(1.0);
                    }
                    _ => event.hurt("minecraft:wither", 100.0),
                }
            }
        }
        let applied = Arc::new(AtomicUsize::new(0));
        let effects = ModEntry::new("test-potions", Box::new(TestMod(applied.clone())));
        mod_platform::__private::install(
            std::iter::once(effects)
                .chain(extra)
                .collect::<Vec<_>>()
                .into(),
        );
        assert!(brewing::register_mod_effects([brewing::ModEffect::new(
            "test:effect",
            0,
            brewing::Category::Neutral,
            "test-potions",
        )]));
        applied
    }

    pub(crate) fn cloud_spawns_during_pass(potion_pass: bool) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.breath_clouds.clear();
            for _ in 0..2 {
                game.spawn_creeper_cloud([0.0, 80.0, 0.0], Vec::new(), Vec::new(), Vec::new(), 0);
            }
            if !potion_pass {
                game.breath_clouds.for_each_mut(|c| c.potion = None);
            }
            let first = game.breath_clouds[0].id;
            let second = game.breath_clouds[1].id;
            let next = game.server.next_entity;
            crate::game::server_side::entity_server::taken_pass_tests::with_spawn(
                |game| {
                    assert!(game.breath_clouds.is_empty());
                    for _ in 0..2 {
                        game.spawn_creeper_cloud(
                            [0.0, 80.0, 0.0],
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            0,
                        );
                    }
                },
                || {
                    if potion_pass {
                        game.tick_potion_clouds();
                    } else {
                        game.tick_breath();
                    }
                },
            );
            assert_eq!(
                game.breath_clouds
                    .iter()
                    .map(|c| (c.id, c.age))
                    .collect::<Vec<_>>(),
                [(next + 1, 0), (next + 2, 0), (first, 1), (second, 1)]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn residency_cloud_load_expiry_bottling_and_late_ids() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.breath_clouds.clear();
            game.ominous_spawners.clear();
            let (tag, _) =
                sunrise_core::nbt::parse_snbt("{Pos:[0d,80d,0d],Duration:2,WaitTime:0,Radius:3f}")
                    .unwrap();
            let potion = game.load_potion_cloud(&tag).unwrap();
            for (age, duration) in [(20, 1), (0, 40)] {
                game.breath_clouds
                    .push(sunrise_gameplay_common::game::dragon_common::Breath {
                        attachment_lifetime: Default::default(),
                        join: Default::default(),
                        id: 0,
                        at: [0.0, 80.0, 0.0],
                        motion: [0.0; 3],
                        fall_distance: 0.0,
                        fire_ticks: 0,
                        first_tick: true,
                        radius: 3.0,
                        age,
                        duration,
                        from_dragon: false,
                        victims: Vec::new(),
                        potion: None,
                    });
            }
            game.add_remote_ominous_item_spawner(0, [0.0, 80.0, 0.0]);
            let before = game.server.next_entity;
            assert_eq!(game.breath_clouds[1].id, 0);
            game.lan_assign_ids();
            assert_eq!(game.ominous_spawners[0].id, before + 1);
            assert_eq!(game.breath_clouds[1].id, before + 2);
            assert_eq!(game.breath_clouds[2].id, before + 3);
            game.tick_breath();
            assert_eq!(
                game.breath_clouds
                    .iter()
                    .map(|c| (c.id, c.age))
                    .collect::<Vec<_>>(),
                [(potion, 0), (before + 3, 1)]
            );
            game.tick_potion_clouds();
            game.tick_potion_clouds();
            assert_eq!(game.breath_clouds.len(), 1);
            assert_eq!(game.breath_clouds[0].age, 1);
            game.breath_clouds.update(0, |c| c.from_dragon = true);
            assert!(
                collect_dragon_breath(
                    Aabb::new(Vec3::new(-0.3, 80.0, -0.3), Vec3::new(0.3, 81.8, 0.3)),
                    &mut game.breath_clouds
                )
                .is_some()
            );
            assert_eq!(game.breath_clouds[0].radius, 2.5);
            game.breath_clouds.clear();
            assert!(game.breath_clouds.is_empty());
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn lifecycle_cloud_commands_and_parked_owners() {
        crate::game::server_side::entity_server::taken_pass_tests::LifecycleFixture {
            seed: |game, entries| {
                game.breath_clouds.clear();
                for &(id, at) in entries {
                    game.spawn_creeper_cloud(at, Vec::new(), Vec::new(), Vec::new(), 0);
                    let i = game.breath_clouds.len() - 1;
                    let mut cloud = game.breath_clouds.remove(i);
                    cloud.id = id;
                    game.breath_clouds.push(cloud);
                }
            },
            ids: |game| game.breath_clouds.iter().map(|c| c.id).collect(),
            target: sunrise_gameplay_common::game::commands_common::Target::Cloud,
            unloads_with_chunk: true,
            remote: None,
        }
        .run();
    }

    #[test]
    #[ignore = "daily"]
    fn taken_breath_tick_keeps_both_spawns_before_survivors() {
        cloud_spawns_during_pass(false);
    }

    #[test]
    #[ignore = "daily"]
    fn taken_potion_tick_keeps_both_spawns_before_survivors() {
        cloud_spawns_during_pass(true);
    }

    #[test]
    #[ignore = "daily"]
    fn typed_cloud_retains_its_loaded_options_for_tracking_and_save() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::tests::typed_cloud_retains_its_loaded_options_for_tracking_and_save",
            "SUNRISE_TEST_TYPED_CLOUD_CASE",
            &["cloud"],
        ) {
            return;
        }
        use sunrise_protocol::entity_data::{ParticleData, Value, serializer};
        use {
            sunrise_gameplay::game::lan_particles::tests::MeasuredParticle,
            sunrise_gameplay::game::server_side::lan_particles::tests::PARTICLE_MAP_READS,
            sunrise_gameplay::game::server_side::lan_particles::tests::test_hub,
        };

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let entries = [mod_api::ModEntry::new("test", Box::new(MeasuredParticle))];
            mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
            sunrise_gameplay_common::game::mods::resolve_particle_providers(&Default::default())
                .unwrap();
            PARTICLE_MAP_READS.set(0);
            let (hub, _, _) = test_hub(
                "CloudParticleOptions",
                vec!["minecraft:entity_effect".into()],
                None,
            );
            game.server.lan = Some(sunrise_gameplay::game::lan::LanHost::for_test(hub));
            let (tag, _) = sunrise_core::nbt::parse_snbt(
                r#"{Pos:[0d,80d,0d],Duration:200,WaitTime:0,Radius:3f,custom_particle:{type:"test:measured",measure:257,ignored:42}}"#,
            ).unwrap();
            let id = game.load_potion_cloud(&tag).unwrap();
            game.tick_potion_clouds();
            assert_eq!(game.breath_clouds[0].age, 1);

            let mut states = sunrise_sim::lan::entity::States::default();
            game.lan_object_states(&mut states, false, 0);
            let cloud = states
                .as_slice()
                .iter()
                .find(|e| e.id as u32 == id)
                .unwrap();
            let (index, ser, Value::Particle(particle)) = cloud
                .meta
                .iter()
                .find(|(index, _, _)| *index == 10)
                .unwrap()
            else {
                panic!("typed cloud was not tracked");
            };
            assert_eq!((*index, *ser), (10, serializer::PARTICLE));
            let ParticleData::Mod { name, options } = particle.data.as_ref().unwrap() else {
                panic!("not a typed particle: {particle:?}");
            };
            assert_eq!(name, "test:measured");
            assert_eq!(options, &[1, 1, 0, 0]);

            let saved = potion_cloud_to_nbt(&game.breath_clouds[0], [1, 2, 3, 4], Some(&tag));
            let particle_tag = saved.get("custom_particle").unwrap();
            assert_eq!(particle_tag.get("measure"), Some(&Tag::Int(257)));
            assert!(
                particle_tag.get("ignored").is_none(),
                "save must use the map encoder, not the input tag"
            );
            assert_eq!(PARTICLE_MAP_READS.get(), 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn owned_effect_ticks_read_live_effects_and_rules_and_subtract_exhaustion() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::tests::owned_effect_ticks_read_live_effects_and_rules_and_subtract_exhaustion",
            "SUNRISE_OWNED_EFFECT_CHILD",
            &["effects"],
        ) {
            return;
        }
        let cancel_expiry = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        mod_platform::testing::with_mods(
            std::sync::Arc::from([
                mod_api::ModEntry::new(
                    "effect-owner",
                    Box::new(FoodEffectMod(cancel_expiry.clone())),
                ),
                mod_api::ModEntry::new("other-mod", Box::new(OtherEffectMod)),
            ]),
            || {
                assert!(brewing::register_mod_effects(
                    [
                        ("test:comfort", "effect-owner"),
                        ("test:nourishment", "effect-owner"),
                        ("other-mod:foreign", "other-mod"),
                    ]
                    .map(|(id, owner)| brewing::ModEffect::new(
                        id,
                        0,
                        brewing::Category::Beneficial,
                        owner,
                    ))
                ));
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                    game.player
                        .as_mut()
                        .unwrap()
                        .set_health(10.0, false)
                        .unwrap();
                    game.player.as_mut().unwrap().health.food.saturation = 0.0;
                    game.potions
                        .effects
                        .add(EffectInstance::new("test:comfort", 80, 0));
                    game.tick_mob_effects();
                    assert_eq!(game.player.as_ref().unwrap().health.health(), 11.0);
                    game.tick_mob_effects();
                    assert_eq!(
                        game.player.as_ref().unwrap().health.health(),
                        11.0,
                        "only each 80 ticks"
                    );
                    game.potions.effects = PlayerEffects::default();
                    game.potions
                        .effects
                        .add(EffectInstance::new("regeneration", 100, 0));
                    game.potions
                        .effects
                        .add(EffectInstance::new("test:comfort", 80, 0));
                    game.tick_mob_effects();
                    assert_eq!(
                        game.player.as_ref().unwrap().health.health(),
                        12.0,
                        "regeneration alone healed"
                    );
                    game.potions.effects = PlayerEffects::default();
                    game.potions
                        .effects
                        .add(EffectInstance::new("regeneration", 1, 0));
                    game.potions
                        .effects
                        .add(EffectInstance::new("test:comfort", 80, 0));
                    game.tick_mob_effects();
                    assert_eq!(
                        game.player.as_ref().unwrap().health.health(),
                        13.0,
                        "an earlier expired regeneration is no longer present"
                    );
                    game.potions.effects = PlayerEffects::default();
                    game.player.as_mut().unwrap().health.food.saturation = 1.0;
                    game.potions
                        .effects
                        .add(EffectInstance::new("test:comfort", 80, 0));
                    game.tick_mob_effects();
                    assert_eq!(
                        game.player.as_ref().unwrap().health.health(),
                        13.0,
                        "saturated players skip comfort"
                    );

                    game.potions.effects = PlayerEffects::default();
                    game.potions
                        .effects
                        .add(EffectInstance::new("test:nourishment", 20, 0));
                    let food = &mut game.player.as_mut().unwrap().health.food;
                    food.food = 18;
                    food.exhaustion = 6.0;
                    game.tick_mob_effects();
                    assert_eq!(
                        game.player.as_ref().unwrap().health.food.exhaustion,
                        6.0,
                        "natural regeneration keeps its exhaustion"
                    );
                    assert!(game.ui.rules.set(
                        "natural_health_regeneration",
                        sunrise_game_data::gamerules::Value::Bool(false)
                    ));
                    game.tick_mob_effects();
                    assert_eq!(game.player.as_ref().unwrap().health.food.exhaustion, 2.0);
                    game.tick_mob_effects();
                    assert_eq!(
                        game.player.as_ref().unwrap().health.food.exhaustion,
                        0.0,
                        "subtract up to four"
                    );
                    game.potions.effects = PlayerEffects::default();
                    game.potions
                        .effects
                        .add(EffectInstance::new("test:nourishment", 20, 1));
                    game.player.as_mut().unwrap().health.food.exhaustion = 1.0;
                    game.tick_mob_effects();
                    assert_eq!(
                        game.player.as_ref().unwrap().health.food.exhaustion,
                        -4.0,
                        "no exhaustion floor"
                    );

                    game.world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                    );
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut mob = Mob::new(12, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                    mob.no_ai = true;
                    mob.health = 5.0;
                    mob.add_effect("test:comfort", 80, 0);
                    mob.add_effect("test:nourishment", 20, 0);
                    game.mobs.push(mob);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        game.mobs[0].health, 6.0,
                        "comfort heals a mob; nourishment has no food to change"
                    );
                    game.mobs[0].retain_effects(|_| false);
                    game.mobs[0].add_effect("minecraft:regeneration", 99, 0);
                    game.mobs[0].add_effect("test:comfort", 80, 0);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        game.mobs[0].health, 6.0,
                        "active regeneration suppresses comfort on a mob"
                    );
                    game.mobs[0].retain_effects(|_| false);
                    game.mobs[0].add_effect("minecraft:regeneration", 1, 0);
                    game.mobs[0].add_effect("test:comfort", 80, 0);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        game.mobs[0].health, 7.0,
                        "earlier mob expiry is visible to comfort"
                    );

                    game.mobs[0].retain_effects(|_| false);
                    game.mobs[0].add_effect("minecraft:regeneration", 1, 0);
                    game.mobs[0].add_effect("test:comfort", 80, 0);
                    cancel_expiry.store(true, std::sync::atomic::Ordering::SeqCst);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        game.mobs[0].health, 7.0,
                        "cancelled expiry still suppresses comfort at duration zero"
                    );
                    assert!(game.mobs[0].has_effect("minecraft:regeneration"));
                    cancel_expiry.store(false, std::sync::atomic::Ordering::SeqCst);
                    game.mobs[0].add_effect("test:comfort", 80, 0);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        game.mobs[0].health, 8.0,
                        "a previously cancelled expiry is asked again before comfort"
                    );

                    game.mobs[0].retain_effects(|_| false);
                    game.mobs[0].add_effect("minecraft:regeneration", 200, 0);
                    game.mobs[0].add_effect("minecraft:regeneration", 1, 1);
                    game.mobs[0].add_effect("test:comfort", 80, 0);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        game.mobs[0].health, 8.0,
                        "a promoted hidden regeneration still suppresses comfort"
                    );
                    let regen = game.mobs[0]
                        .extra
                        .effects
                        .iter()
                        .find(|effect| effect.id == "minecraft:regeneration")
                        .unwrap();
                    assert_eq!((regen.duration, regen.amplifier), (199, 0));
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn mod_tick_heals_and_feeds_player() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::tests::mod_tick_heals_and_feeds_player",
            "SUNRISE_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod([]);
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let player = game.player.as_mut().unwrap();
            player.set_health(18.0, false).unwrap();
            player.health.food.food = 14;
            player.health.food.saturation = 2.0;
            let id = brewing::effect("test:effect").unwrap().id;
            game.potions.effects.add(EffectInstance::new(id, 10, 0));
            game.tick_mob_effects();
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.health.health(), 20.0);
            assert_eq!(player.health.food.food, 18);
            assert_eq!(player.health.food.saturation, 6.0);
            assert_eq!(player.health.food.exhaustion, 1.5);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mod_tick_magic_respects_player_hit_frames() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::tests::mod_tick_magic_respects_player_hit_frames",
            "SUNRISE_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod([]);
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let id = brewing::effect("test:effect").unwrap().id;
            game.potions.effects.add(EffectInstance::new(id, 10, 1));
            game.tick_mob_effects();
            assert_eq!(game.player.as_ref().unwrap().health.health(), 17.0);
            game.tick_mob_effects();
            assert_eq!(game.player.as_ref().unwrap().health.health(), 17.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mod_tick_kills_mob_through_death_and_drops() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::tests::mod_tick_kills_mob_through_death_and_drops",
            "SUNRISE_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod([]);
        super::super::native_entity_refs::tests::with_game(|game| {
            // Death drops read the server's loot, which a world open loads.
            game.load_server_loot();
            use std::sync::Arc;
            use sunrise_level::chunk::Chunk;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(12, kind, Vec3::new(1.0, 64.0, 1.0), 0.0);
            mob.extra
                .set_main_hand(Some(Arc::from("minecraft:iron_sword")));
            mob.extra.drop_chances = Some(Box::new([2.0; 8]));
            mob.add_effect(brewing::effect("test:effect").unwrap().id, 10, 2);
            game.mobs.push(mob);
            game.tick_creature_entities().unwrap();
            assert!(!game.mobs[0].alive());
            assert!(
                game.dropped
                    .iter()
                    .any(|item| &*item.stack.item == "minecraft:iron_sword")
            );
            for _ in 0..20 {
                game.tick_creature_entities().unwrap();
            }
            assert!(game.mobs.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mod_tick_sees_prior_regeneration() {
        // Mod effects are frozen process-wide: another fixture may have registered first.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::tests::mod_tick_sees_prior_regeneration",
            "SUNRISE_POTION_EFFECT_CHILD",
            &["effect"],
        ) {
            return;
        }
        effect_test_mod([]);
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player
                .as_mut()
                .unwrap()
                .set_health(1.0, false)
                .unwrap();
            game.potions
                .effects
                .add(EffectInstance::new("regeneration", 50, 0));
            let id = brewing::effect("test:effect").unwrap().id;
            game.potions.effects.add(EffectInstance::new(id, 10, 3));
            game.tick_mob_effects();
            assert_eq!(game.player.as_ref().unwrap().health.health(), 1.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_effect_added_event_and_part_on_effect_added_contract() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::tests::mob_effect_added_event_and_part_on_effect_added_contract",
            "SUNRISE_EFFECT_ADDED_CHILD",
            &["added"],
        ) {
            return;
        }
        #[derive(Debug, PartialEq, Eq)]
        enum Record {
            Bus {
                entity: u32,
                effect: Option<String>,
                old: Option<(i32, i32)>,
                new: (i32, i32),
                live: Option<(i32, i32)>,
                source: Option<u32>,
            },
            Part {
                entity: u32,
                effect: Option<String>,
                amplifier: i32,
                live: Option<(i32, i32)>,
            },
        }
        #[derive(Clone)]
        struct Observer {
            log: Arc<Mutex<Vec<Record>>>,
            deny: Arc<AtomicBool>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::MobEffectPart for Observer {
            fn on_effect_added(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::OnEffectAddedEvent,
            ) {
                let live = event
                    .entity
                    .effect(host, event.effect)
                    .map(|e| (e.duration, e.amplifier));
                self.log.lock().unwrap().push(Record::Part {
                    entity: event.entity.entity().id(),
                    effect: event.effect.id(host).map(str::to_owned),
                    amplifier: event.amplifier,
                    live,
                });
            }
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Observer {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let effect = r.effect_key("test:comfort");
                r.mob_effect_part(&effect, self.clone()).unwrap();
            }
            fn mob_effect_applicable(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mut mod_api::MobEffectApplicableEvent,
            ) -> Option<mod_api::EffectApplicable> {
                self.deny
                    .load(Ordering::SeqCst)
                    .then_some(mod_api::EffectApplicable::Deny)
            }
            fn mob_effect_added(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::MobEffectAddedEvent<'_>,
            ) {
                let live = event
                    .entity
                    .effect(host, event.new_effect.id)
                    .map(|e| (e.duration, e.amplifier));
                self.log.lock().unwrap().push(Record::Bus {
                    entity: event.entity.entity().id(),
                    effect: event.new_effect.id.id(host).map(str::to_owned),
                    old: event.old_effect.map(|e| (e.duration, e.amplifier)),
                    new: (event.new_effect.duration, event.new_effect.amplifier),
                    live,
                    source: event.source.map(|s| s.id()),
                });
            }
        }

        let log = Arc::new(Mutex::new(Vec::new()));
        let deny = Arc::new(AtomicBool::new(false));
        let observer = Observer {
            log: log.clone(),
            deny: deny.clone(),
        };
        mod_platform::testing::with_mods(
            Arc::from([mod_api::ModEntry::new("effect-owner", Box::new(observer))]),
            || {
                assert!(brewing::register_mod_effects([brewing::ModEffect::new(
                    "test:comfort",
                    0,
                    brewing::Category::Beneficial,
                    "effect-owner",
                )]));
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                    game.player.as_mut().unwrap().entity_id = 20;
                    super::super::native_entity_refs::tests::bind_local_player(game, 20, 123);
                    let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                    let spider = game
                        .res
                        .mob_types()
                        .get("minecraft:spider")
                        .unwrap()
                        .clone();
                    let wither = game
                        .res
                        .mob_types()
                        .get("minecraft:wither")
                        .unwrap()
                        .clone();
                    game.mobs.push(Mob::new(21, cow, Vec3::ZERO, 0.0));
                    game.mobs.push(Mob::new(22, spider, Vec3::ZERO, 0.0));
                    game.mobs.push(Mob::new(23, wither, Vec3::ZERO, 0.0));

                    sunrise_gameplay_common::game::test_fixtures::fixture_resident_column(
                        &mut game.world,
                        (0, 0),
                    );
                    let player_source = game.entity(20);
                    let cow_source = game.entity(21);

                    // 1. Vanilla effect on player: first add and unchanged reapplication both fire
                    //    the global bus event pre-mutation, with source and old/new snapshots.
                    assert!(
                        game.add_player_effect_from(
                            EffectInstance::new("speed", 200, 0),
                            cow_source,
                        )
                    );
                    assert!(!game.add_player_effect(EffectInstance::new("speed", 100, 0)));
                    assert_eq!(
                        std::mem::take(&mut *log.lock().unwrap()),
                        vec![
                            Record::Bus {
                                entity: 20,
                                effect: Some("minecraft:speed".into()),
                                old: None,
                                new: (200, 0),
                                live: None,
                                source: Some(21),
                            },
                            Record::Bus {
                                entity: 20,
                                effect: Some("minecraft:speed".into()),
                                old: Some((200, 0)),
                                new: (100, 0),
                                live: Some((200, 0)),
                                source: None,
                            },
                        ]
                    );

                    // 2. Mod effect on mob: first add fires bus pre-mutation then Part post-insert;
                    //    stronger and unchanged reapplications fire bus pre-mutation only.
                    let comfort = brewing::effect("test:comfort").unwrap();
                    assert!(game.add_mob_effect(
                        21,
                        EffectInstance::from_def(comfort, 100, 1),
                        player_source,
                    ));
                    assert!(game.add_mob_effect(
                        21,
                        EffectInstance::from_def(comfort, 200, 2),
                        None,
                    ));
                    assert!(!game.add_mob_effect(
                        21,
                        EffectInstance::from_def(comfort, 50, 0),
                        None,
                    ));
                    assert_eq!(
                        std::mem::take(&mut *log.lock().unwrap()),
                        vec![
                            Record::Bus {
                                entity: 21,
                                effect: Some("test:comfort".into()),
                                old: None,
                                new: (100, 1),
                                live: None,
                                source: Some(20),
                            },
                            Record::Part {
                                entity: 21,
                                effect: Some("test:comfort".into()),
                                amplifier: 1,
                                live: Some((100, 1)),
                            },
                            Record::Bus {
                                entity: 21,
                                effect: Some("test:comfort".into()),
                                old: Some((100, 1)),
                                new: (200, 2),
                                live: Some((100, 1)),
                                source: None,
                            },
                            Record::Bus {
                                entity: 21,
                                effect: Some("test:comfort".into()),
                                old: Some((200, 2)),
                                new: (50, 0),
                                live: Some((200, 2)),
                                source: None,
                            },
                        ]
                    );

                    // 3. Refused effects (spider poison, WitherBoss.addEffect, mob_effect_applicable
                    //    Deny) and load_effect emit neither the bus event nor the Part callback.
                    assert!(!game.add_mob_effect(22, EffectInstance::new("poison", 100, 0), None));
                    assert!(!game.add_mob_effect(
                        23,
                        EffectInstance::from_def(comfort, 100, 0),
                        None,
                    ));
                    deny.store(true, Ordering::SeqCst);
                    assert!(!game.add_player_effect(EffectInstance::from_def(comfort, 100, 0)));
                    deny.store(false, Ordering::SeqCst);
                    game.mobs[0].load_effect(sunrise_sim::mobs::StoredEffect::new(
                        "minecraft:regeneration",
                        100,
                        0,
                    ));
                    assert!(log.lock().unwrap().is_empty());
                });
            },
        );
    }
}

#[cfg(test)]
#[path = "potions/cloud_water_tests_server.rs"]
pub(crate) mod cloud_water_tests;

#[cfg(test)]
pub(crate) mod sound_on_added_tests {
    use sunrise_level::effects::EffectInstance;

    /// `LivingEntity.addEffect` → `MobEffect.onEffectAdded`: Bad Omen, Trial
    /// Omen and Raid Omen sound once when they are new on the player, at its
    /// feet; a renewal and any other effect are silent (#11373).
    #[test]
    #[ignore = "daily"]
    fn a_new_omen_sounds_once_at_the_feet() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(10.5, 64.0, -3.25),
                )
                .unwrap(),
            );
            game.start_audio_capture();
            for (id, amplifier) in [
                ("bad_omen", 0),
                ("bad_omen", 1),
                ("speed", 0),
                ("trial_omen", 0),
                ("raid_omen", 0),
            ] {
                game.add_player_effect(EffectInstance::new(id, 2400, amplifier));
            }
            let played: Vec<(String, [f64; 3])> = std::mem::take(&mut game.server.pending_sounds)
                .into_iter()
                .map(|s| (s.event, s.position))
                .collect();
            let feet = [10.5, 64.0, -3.25];
            assert_eq!(
                played,
                [
                    ("minecraft:event.mob_effect.bad_omen".to_owned(), feet),
                    ("minecraft:event.mob_effect.trial_omen".to_owned(), feet),
                    ("minecraft:event.mob_effect.raid_omen".to_owned(), feet),
                ]
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod xp_bottle_tests {
    use super::*;

    /// `ThrownExperienceBottle.onHit` uses `awardWithDirection`: every fresh
    /// orb starts 0.25 out from the face it hit, or back along the bottle's
    /// motion after an entity hit, moving that way (#11388).
    #[test]
    #[ignore = "daily"]
    fn bottle_orbs_start_away_from_what_it_hit() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let bottle = ItemStack::new("minecraft:experience_bottle", 1);
            let at = Vec3::new(8.5, 70.5, 8.5);
            // (hit face, the bottle's motion, the axis and sign the orbs leave by)
            for (face, motion, axis, sign) in [
                (
                    Some(([9, 70, 8], Direction::West)),
                    Vec3::new(1.0, 0.0, 0.0),
                    0,
                    -1.0,
                ),
                (None, Vec3::new(0.0, 0.0, 1.0), 2, -1.0),
            ] {
                game.orbs.clear();
                game.splash(
                    &bottle,
                    at,
                    None,
                    face,
                    0,
                    0,
                    &PotionOwner::default(),
                    false,
                    motion,
                );
                assert!(!game.orbs.is_empty());
                let at = [at.x, at.y, at.z];
                for o in game.orbs.iter() {
                    let p = [o.position.x, o.position.y, o.position.z];
                    let v = [o.velocity.x, o.velocity.y, o.velocity.z];
                    assert!((p[axis] - (at[axis] + sign * 0.25)).abs() < 1e-9, "{p:?}");
                    assert!(v[axis] * sign >= 0.0, "{v:?}");
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "potions/potion_contents_tests.rs"]
pub(crate) mod potion_contents_tests;

#[cfg(test)]
pub(crate) mod mob_absorption_max_tests {
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, position::ChunkPos};
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h203-{:?}", std::thread::current().id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("scoreboard objectives add h203 dummy"));
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    fn pig(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([203, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live pig with UUID {n}");
        i[0]
    }
    /// `/attribute ... max_absorption get`, read back through a score.
    fn max_absorption(game: &mut sunrise_gameplay::game::ServerGame<'_>, tag: &str) -> i32 {
        assert!(game.run_command(&format!(
            "execute store result score #max h203 run attribute @e[tag={tag},limit=1] minecraft:max_absorption get"
        )));
        (0..=64)
            .find(|v| game.run_command_quietly(&format!("execute if score #max h203 matches {v}")))
            .unwrap_or(-1)
    }

    /// Expected values from the official 26.3 server: Absorption adds
    /// 4 * (amplifier + 1) to max_absorption; its add raises the amount to at
    /// least that much within the maximum; when it goes, the amount is
    /// clamped to the maximum left (LivingEntity.onAttributeUpdated), so a
    /// base-20 mob keeps it and a base-0 one loses it.
    #[test]
    #[ignore = "daily"]
    fn absorption_follows_the_max_absorption_it_adds_and_leaves() {
        with_game(|game| {
            // name, UUID word, base max_absorption, amplifier, duration seconds
            let cases = [
                ("b0_clear", 1, None, 1, 100),
                ("b0_expiry", 2, None, 0, 1),
                ("b20_clear", 3, Some(20), 1, 100),
                ("b20_expiry", 4, Some(20), 0, 1),
                ("b20_upgrade", 5, Some(20), 0, 100),
            ];
            for (name, n, base, amp, secs) in cases {
                let tag = format!("h203_{name}");
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 4.5 {{UUID:[I;203,0,0,{n}],Tags:[\"{tag}\"],PersistenceRequired:1b,Silent:1b,NoAI:1b,Health:10f}}",
                    2 * n
                )));
                if let Some(b) = base {
                    assert!(game.run_command(&format!(
                        "attribute @e[tag={tag},limit=1] minecraft:max_absorption base set {b}"
                    )));
                }
                assert!(game.run_command(&format!(
                    "effect give @e[tag={tag},limit=1] minecraft:absorption {secs} {amp}"
                )));
            }
            assert!(game.run_command(
                "effect give @e[tag=h203_b20_upgrade,limit=1] minecraft:absorption 100 1"
            ));
            let mut failures = Vec::new();
            let mut expect = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                              phase: &str,
                              rows: [(&str, i32, f32, i32); 5]| {
                for (name, n, amount, max) in rows {
                    let i = pig(game, n);
                    if game.mobs[i].absorption != amount {
                        failures.push(format!(
                            "{phase} {name}: amount {}, expected {amount}",
                            game.mobs[i].absorption
                        ));
                    }
                    let got = max_absorption(game, &format!("h203_{name}"));
                    if got != max {
                        failures.push(format!(
                            "{phase} {name}: max_absorption {got}, expected {max}"
                        ));
                    }
                }
            };
            expect(
                game,
                "added",
                [
                    ("b0_clear", 1, 8.0, 8),
                    ("b0_expiry", 2, 4.0, 4),
                    ("b20_clear", 3, 8.0, 28),
                    ("b20_expiry", 4, 4.0, 24),
                    ("b20_upgrade", 5, 8.0, 28),
                ],
            );
            for tag in ["h203_b0_clear", "h203_b20_clear"] {
                assert!(game.run_command(&format!(
                    "effect clear @e[tag={tag},limit=1] minecraft:absorption"
                )));
            }
            expect(
                game,
                "cleared",
                [
                    ("b0_clear", 1, 0.0, 0),
                    ("b0_expiry", 2, 4.0, 4),
                    ("b20_clear", 3, 8.0, 20),
                    ("b20_expiry", 4, 4.0, 24),
                    ("b20_upgrade", 5, 8.0, 28),
                ],
            );
            for _ in 0..25 {
                game.tick_world(true).unwrap();
            }
            expect(
                game,
                "expired",
                [
                    ("b0_clear", 1, 0.0, 0),
                    ("b0_expiry", 2, 0.0, 0),
                    ("b20_clear", 3, 8.0, 20),
                    ("b20_expiry", 4, 4.0, 20),
                    ("b20_upgrade", 5, 8.0, 28),
                ],
            );
            assert!(failures.is_empty(), "mob absorption maximum: {failures:#?}");
        });
    }
}

#[cfg(test)]
pub(crate) mod mob_health_boost_tests {
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, position::ChunkPos};
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h205-{:?}", std::thread::current().id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    fn pig(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([205, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live pig with UUID {n}");
        i[0]
    }
    fn health(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> f32 {
        game.mobs[pig(game, n)].health
    }
    fn boost(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> Option<(i32, i32)> {
        let i = pig(game, n);
        game.mobs[i]
            .extra
            .effects
            .iter()
            .find(|e| e.id == "minecraft:health_boost")
            .map(|e| (e.duration, e.amplifier))
    }

    /// Expected values from the official 26.3 server: Health Boost II adds 8
    /// to max_health without healing; Instant Health heals up to it on the next
    /// tick; removing the effect (`/effect clear`, or running out) clamps
    /// Health to the maximum without it (LivingEntity.onAttributeUpdated).
    #[test]
    #[ignore = "daily"]
    fn health_boost_heals_to_its_maximum_and_clamps_when_it_goes() {
        with_game(|game| {
            // name, UUID word, Health, setup
            let cases: [(&str, i32, f32, &[&str]); 5] = [
                (
                    "boost",
                    1,
                    10.0,
                    &["effect give @e[tag=T,limit=1] minecraft:health_boost 100 1"],
                ),
                (
                    "expiry",
                    2,
                    10.0,
                    &["effect give @e[tag=T,limit=1] minecraft:health_boost 2 1"],
                ),
                ("noboost", 3, 10.0, &[]),
                ("heal", 5, 5.0, &[]),
                (
                    "base",
                    6,
                    10.0,
                    &["attribute @e[tag=T,limit=1] minecraft:max_health base set 14"],
                ),
            ];
            for (name, n, h, setup) in cases {
                let tag = format!("h205_{name}");
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 4.5 {{UUID:[I;205,0,0,{n}],Tags:[\"{tag}\"],PersistenceRequired:1b,Silent:1b,NoAI:1b,Health:{h}f}}",
                    2 * n
                )));
                for c in setup {
                    assert!(game.run_command(&c.replace('T', &tag)), "{name}: {c}");
                }
            }
            // Setup controls: the effect is there, nothing healed on the add.
            assert_eq!(boost(game, 1), Some((2000, 1)));
            assert_eq!(boost(game, 2), Some((40, 1)));
            for (_, n, h, _) in cases {
                assert_eq!(health(game, n), h, "UUID {n}: no heal on the add");
            }
            for (name, n, _, _) in cases {
                let level = if name == "heal" { 0 } else { 1 };
                assert!(game.run_command(&format!(
                    "effect give @e[tag=h205_{name},limit=1] minecraft:instant_health 1 {level}"
                )));
                let _ = n;
            }
            game.tick_world(true).unwrap();
            // The one tick: the boosts counted down once.
            assert_eq!(boost(game, 1), Some((1999, 1)), "the mobs ticked once");
            let mut failures = Vec::new();
            let check = |game: &sunrise_gameplay::game::ServerGame<'_>,
                         failures: &mut Vec<String>,
                         phase: &str,
                         expected: [(i32, f32); 5]| {
                for (n, h) in expected {
                    if health(game, n) != h {
                        failures.push(format!(
                            "{phase}: UUID {n} Health {}, expected {h}",
                            health(game, n)
                        ));
                    }
                }
            };
            check(
                game,
                &mut failures,
                "healed",
                [(1, 18.0), (2, 18.0), (3, 10.0), (5, 9.0), (6, 14.0)],
            );
            assert!(
                game.run_command("effect clear @e[tag=h205_boost,limit=1] minecraft:health_boost")
            );
            check(
                game,
                &mut failures,
                "cleared",
                [(1, 10.0), (2, 18.0), (3, 10.0), (5, 9.0), (6, 14.0)],
            );
            for _ in 0..40 {
                game.tick_world(true).unwrap();
            }
            if boost(game, 2).is_some() {
                failures.push("expiry: the effect did not run out in 40 ticks".into());
            }
            check(
                game,
                &mut failures,
                "expired",
                [(1, 10.0), (2, 10.0), (3, 10.0), (5, 9.0), (6, 14.0)],
            );
            assert!(failures.is_empty(), "mob health boost: {failures:#?}");
        });
    }

    /// Sulfur cube sizing writes an attribute base (`set_size` -> `set_base`),
    /// so Health Boost adds to that base without healing, as in the command
    /// parity corpus. Taming still owns the wolf's cached maximum outside the
    /// model; that maximum and its current health remain unchanged.
    #[test]
    #[ignore = "daily"]
    fn health_boost_adds_to_size_attributes_and_keeps_the_tame_maximum() {
        with_game(|game| {
            // tag, UUID word, summon, amplifier
            let cases = [
                // size 3: attribute base and cache 12; Health Boost II adds 8
                ("h205_cube", 20, "sulfur_cube 4.5 200 8.5 {Size:2", 1),
                // size 5: attribute base and cache 20; Health Boost II adds 8
                ("h205_cube_equal", 22, "sulfur_cube 6.5 200 8.5 {Size:4", 1),
                // tame: max-health base 40; Health Boost II adds 8
                ("h205_wolf", 21, "wolf 8.5 200 8.5 {", 1),
                // tame: Health Boost VIII adds 32 to the tame base
                ("h205_wolf_equal", 23, "wolf 10.5 200 8.5 {", 7),
            ];
            let mut ids = Vec::new();
            for (tag, n, summon, _) in cases {
                let sep = if summon.ends_with('{') { "" } else { "," };
                assert!(game.run_command(&format!(
                    "summon minecraft:{summon}{sep}UUID:[I;205,0,0,{n}],Tags:[\"{tag}\"],NoAI:1b,PersistenceRequired:1b,Silent:1b}}"
                )));
                let i = pig(game, n);
                if tag.starts_with("h205_wolf") {
                    sunrise_sim::mobs::tame::set_tame(&mut game.mobs[i], true);
                }
                ids.push(i);
            }
            let state = |game: &sunrise_gameplay::game::ServerGame<'_>| -> Vec<(f32, f32)> {
                ids.iter()
                    .map(|&i| (game.mobs[i].attrs.max_health, game.mobs[i].health))
                    .collect()
            };
            let before = state(game);
            assert_eq!(
                before.iter().map(|s| s.0).collect::<Vec<_>>(),
                [12.0, 20.0, 40.0, 40.0]
            );
            assert_eq!(
                sunrise_sim::mobs::finalize::max_health(&game.mobs[ids[1]]),
                20.0,
                "sizing installs the cube's actual max-health attribute base"
            );
            for (tag, _, _, amp) in cases {
                assert!(game.run_command(&format!(
                    "effect give @e[tag={tag},limit=1] minecraft:health_boost 100 {amp}"
                )));
            }
            assert_eq!(
                sunrise_sim::mobs::finalize::max_health(&game.mobs[ids[3]]),
                72.0,
                "health boost adds to the tamed wolf's attribute base"
            );
            assert_eq!(
                state(game),
                [
                    (20.0, before[0].1),
                    (28.0, before[1].1),
                    (48.0, before[2].1),
                    (72.0, before[3].1),
                ],
                "size and tame attributes gain the modifier without healing"
            );
            for (tag, _, _, _) in cases {
                assert!(game.run_command(&format!(
                    "effect clear @e[tag={tag},limit=1] minecraft:health_boost"
                )));
            }
            assert_eq!(state(game), before, "and after the clear");
        });
    }
}

#[cfg(test)]
pub(crate) mod depleted_absorption_tests {
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, position::ChunkPos};
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h204-{:?}", std::thread::current().id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    /// The mob in the level with UUID word `n`.
    fn pig(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([204, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live pig with UUID {n}");
        i[0]
    }
    /// Absorption's remaining ticks on the mob, if it has the effect.
    fn absorption(game: &sunrise_gameplay::game::ServerGame<'_>, i: usize) -> Option<i32> {
        game.mobs[i]
            .extra
            .effects
            .iter()
            .find(|e| e.id == "minecraft:absorption")
            .map(|e| e.duration)
    }

    /// Expected values from the official 26.3 server:
    /// AbsorptionMobEffect.applyEffectTick is false once the amount is 0, so
    /// MobEffectInstance.tickServer drops the effect on the mob's next tick,
    /// before the duration counts down; with any amount left it stays.
    #[test]
    #[ignore = "daily"]
    fn a_mob_loses_absorption_on_the_tick_after_it_is_used_up() {
        with_game(|game| {
            // name, UUID word, extra NBT, effect, hit
            let cases = [
                ("depleted", 1, "NoAI:1b", true, Some(4)),
                ("kept", 2, "NoAI:1b", true, None),
                ("partial", 3, "NoAI:1b", true, Some(2)),
                ("none", 4, "NoAI:1b", false, Some(4)),
                ("refused", 5, "NoAI:1b,Invulnerable:1b", true, Some(4)),
                ("ai", 6, "", true, Some(4)),
            ];
            for (name, n, extra, effect, _) in cases {
                let extra = if extra.is_empty() {
                    String::new()
                } else {
                    format!(",{extra}")
                };
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 4.5 {{UUID:[I;204,0,0,{n}],Tags:[\"h204_{name}\"],PersistenceRequired:1b,Silent:1b,Health:10f{extra}}}",
                    2 * n
                )));
                if effect {
                    assert!(game.run_command(&format!(
                        "effect give @e[tag=h204_{name},limit=1] minecraft:absorption 100 0"
                    )));
                }
            }
            for (name, _, _, _, hit) in cases {
                if let Some(hit) = hit {
                    let accepted = game.run_command(&format!(
                        "damage @e[tag=h204_{name},limit=1] {hit} minecraft:generic"
                    ));
                    assert_eq!(accepted, name != "refused", "{name}: damage result");
                }
            }
            // Setup controls, independent of the tick: amount, health, effect.
            for (name, n, _, effect, hit) in cases {
                let i = pig(game, n);
                let m = &game.mobs[i];
                let amount = match (effect, hit) {
                    (false, _) => 0.0,
                    (true, Some(h)) if name != "refused" => 4.0 - h as f32,
                    _ => 4.0,
                };
                assert_eq!(m.absorption, amount, "{name}: absorption after the hit");
                let health = if !effect && hit.is_some() { 6.0 } else { 10.0 };
                assert_eq!(m.health, health, "{name}: health after the hit");
                assert_eq!(
                    absorption(game, i),
                    effect.then_some(2000),
                    "{name}: effect before the tick"
                );
            }

            // One level tick (`tick_world` runs after `tick` has advanced the
            // clock): the kept effect counting 2000 -> 1999 shows the mobs ticked once.
            let time = game.world.game_time;
            game.tick_world(true).unwrap();
            assert_eq!(
                game.world.game_time, time,
                "the level tick leaves the clock to `tick`"
            );
            assert_eq!(
                absorption(game, pig(game, 2)),
                Some(1999),
                "the mobs ticked exactly once"
            );
            let mut failures = Vec::new();
            for (name, n, _, effect, _) in cases {
                let i = pig(game, n);
                let expected = match name {
                    "depleted" | "ai" => None,
                    _ if effect => Some(1999),
                    _ => None,
                };
                if absorption(game, i) != expected {
                    failures.push(format!(
                        "{name}: effect {:?}, expected {expected:?}",
                        absorption(game, i)
                    ));
                }
                let saved = game
                    .command_target_nbt(Target::Mob(game.mobs[i].id))
                    .unwrap();
                let listed = matches!(saved.get("active_effects"), Some(sunrise_core::nbt::Tag::List(l)) if !l.is_empty());
                if listed != expected.is_some() {
                    failures.push(format!("{name}: saved active_effects {listed}"));
                }
            }
            for (name, n, _, _, _) in [cases[0], cases[5]] {
                let i = pig(game, n);
                if game.mobs[i].absorption != 0.0 || game.mobs[i].health != 10.0 {
                    failures.push(format!("{name}: amount/health changed by the removal"));
                }
            }
            assert!(failures.is_empty(), "depleted absorption: {failures:#?}");
        });
    }

    /// The same for infinite effects (`MobEffectInstance.tickServer` asks
    /// `applyEffectTick` by the mob's tickCount): a used-up infinite
    /// Absorption goes, an unused one stays infinite, and another infinite
    /// effect is untouched.
    #[test]
    #[ignore = "daily"]
    fn infinite_absorption_goes_only_once_used_up() {
        with_game(|game| {
            // name, UUID word, infinite effects, hit
            let cases: [(&str, i32, &[&str], bool); 4] = [
                ("inf_depleted", 11, &["absorption"], true),
                ("inf_kept", 12, &["absorption"], false),
                ("inf_other", 13, &["speed"], true),
                ("inf_both", 14, &["speed", "absorption"], true),
            ];
            for (name, n, effects, _) in cases {
                assert!(game.run_command(&format!(
                    "summon minecraft:pig {}.5 200 8.5 {{UUID:[I;204,0,0,{n}],Tags:[\"h204_{name}\"],PersistenceRequired:1b,Silent:1b,Health:10f,NoAI:1b}}",
                    n - 9
                )));
                for effect in effects {
                    assert!(game.run_command(&format!(
                        "effect give @e[tag=h204_{name},limit=1] minecraft:{effect} infinite 0"
                    )));
                }
            }
            for (name, _, _, hit) in cases {
                if hit {
                    assert!(game.run_command(&format!(
                        "damage @e[tag=h204_{name},limit=1] 4 minecraft:generic"
                    )));
                }
            }
            let effect = |game: &sunrise_gameplay::game::ServerGame<'_>, i: usize, id: &str| {
                game.mobs[i]
                    .extra
                    .effects
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.duration)
            };
            for (name, n, effects, hit) in cases {
                let i = pig(game, n);
                let has = effects.contains(&"absorption");
                assert_eq!(
                    game.mobs[i].absorption,
                    if has && !hit { 4.0 } else { 0.0 },
                    "{name}: amount"
                );
                assert_eq!(
                    game.mobs[i].health,
                    if hit && !has { 6.0 } else { 10.0 },
                    "{name}: health"
                );
                assert_eq!(
                    effect(game, i, "minecraft:absorption"),
                    has.then_some(-1),
                    "{name}: before"
                );
            }
            let mut failures = Vec::new();
            for tick in 1..=2 {
                game.tick_world(true).unwrap();
                for (name, n, effects, hit) in cases {
                    let i = pig(game, n);
                    let absorption = (effects.contains(&"absorption") && !hit).then_some(-1);
                    let speed = effects.contains(&"speed").then_some(-1);
                    let got = (
                        effect(game, i, "minecraft:absorption"),
                        effect(game, i, "minecraft:speed"),
                    );
                    if got != (absorption, speed) {
                        failures.push(format!(
                            "{name} tick {tick}: (absorption, speed) {got:?}, expected {:?}",
                            (absorption, speed)
                        ));
                    }
                }
            }
            assert!(failures.is_empty(), "infinite absorption: {failures:#?}");
        });
    }
}

#[cfg(test)]
pub(crate) mod arrow_contents {
    //! Actual in-memory Game potion paths. No run loop, world load or server startup.
    use super::*;
    use serde_json::{Value, json};
    use std::{path::PathBuf, sync::Arc, time::Instant};
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role,
    };

    fn stack(row: &Value) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:tipped_arrow", 1);
        if let Some(potion) = row["potion"].as_str() {
            stack = stack.with_potion(&format!("minecraft:{potion}"));
        }
        let effects = row["custom"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                Tag::compound([
                    (
                        "id",
                        Tag::String(
                            format!("minecraft:{}", e["id"].as_str().unwrap())
                                .as_str()
                                .into(),
                        ),
                    ),
                    ("duration", Tag::Int(e["duration"].as_i64().unwrap() as i32)),
                    ("amplifier", Tag::Int(e["amp"].as_i64().unwrap() as i32)),
                    (
                        "ambient",
                        Tag::Byte(i8::from(e["ambient"].as_bool().unwrap())),
                    ),
                    (
                        "show_particles",
                        Tag::Byte(i8::from(e["visible"].as_bool().unwrap())),
                    ),
                ])
            })
            .collect();
        stack
            .data_mut()
            .potion_extra
            .push(("custom_effects".into(), Tag::List(effects)));
        if let Some(color) = row["color"].as_i64() {
            stack
                .data_mut()
                .potion_extra
                .push(("custom_color".into(), Tag::Int(color as i32)));
        }
        if let Some(name) = row["name"].as_str() {
            stack
                .data_mut()
                .potion_extra
                .push(("custom_name".into(), Tag::String(name.into())));
        }
        stack
    }
    #[test]
    #[ignore = "explicit SUNRISE_TEST_RESOURCES and isolated potion oracle input/output"]
    fn actual_arrow_contents_match_pinned_methods() {
        run_actual_arrow_contents_match_pinned_methods();
    }

    pub(crate) fn run_actual_arrow_contents_match_pinned_methods() {
        let root =
            PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect("explicit resources"));
        let input =
            PathBuf::from(std::env::var_os("SUNRISE_POTION_INPUT").expect("explicit input"));
        let output =
            PathBuf::from(std::env::var_os("SUNRISE_POTION_OUTPUT").expect("explicit output"));
        let expected: Vec<Value> = serde_json::from_slice(
            &std::fs::read(
                std::env::var_os("SUNRISE_POTION_EXPECTED").expect("pinned method output"),
            )
            .unwrap(),
        )
        .unwrap();
        let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(std::env::current_dir().unwrap());
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        assert_eq!(rows.len(), 5, "all five arrow cases must execute");
        let mut results = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let id = row["id"].clone();
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.5, 64.5, 0.5),
                None,
            );
            let st = arrow.arrow.as_mut().unwrap();
            st.pickup_item = stack(row);
            st.in_ground = true;
            st.in_ground_time = 598;
            st.last_state = Some(stone);
            arrow.stuck = true;
            let rain = |_| false;
            let scene = game.arrow_scene(&rain);
            sunrise_sim::entities::arrows::tick(&mut arrow, &scene, &[]);
            let st = arrow.arrow.as_ref().unwrap();
            let item599 = st
                .pickup_item
                .item
                .trim_start_matches("minecraft:")
                .to_owned();
            let color599 = st.potion_color().map_or(-1, |v| v as i32);
            sunrise_sim::entities::arrows::tick(&mut arrow, &scene, &[]);
            let st = arrow.arrow.as_ref().unwrap();
            results.push(json!({"id":id,"item599":item599,"color599":color599,"item600":st.pickup_item.item.trim_start_matches("minecraft:"),"color600":st.potion_color().map_or(-1,|v|v as i32)}));
            assert_eq!(
                results.last().unwrap(),
                &expected[index],
                "actual arrow tick: {}",
                id
            );
        }
        std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}

#[cfg(test)]
pub(crate) mod tick_effects_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_level::chunk::Chunk;

    #[test]
    #[ignore = "daily"]
    fn bad_omen_production_tick_stops_on_raid_omen_insert_and_borrows_hidden_chains() {
        struct CountExpired(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for CountExpired {
            fn mob_effect_expired(
                &self,
                _: &dyn mod_api::Host,
                _: &mut mod_api::MobEffectExpiredEvent,
            ) -> mod_api::Flow {
                self.0.fetch_add(1, Ordering::Relaxed);
                mod_api::Flow::Continue
            }
        }

        let expired_calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "row6-prod-test",
            Box::new(CountExpired(expired_calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                    .unwrap();
                game.options.dedicated = Some(Vec::new());
                for cx in -1..=1 {
                    for cz in -1..=1 {
                        game.world
                            .chunks
                            .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                    }
                }
                let registry = game.pack.assets.registry;
                let foot = registry.default_state("minecraft:white_bed").unwrap();
                let mut head_state = registry.state(foot).unwrap().clone();
                head_state.properties.insert("part".into(), "head".into());
                let head = registry.id(&head_state).unwrap();
                game.world.set(2, 64, 2, foot).unwrap();
                game.world.set(2, 64, 1, head).unwrap();
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:villager")
                    .unwrap()
                    .clone();
                let mut villager = Mob::new(162, kind, Vec3::new(3.5, 64.0, 2.5), 0.0);
                villager.no_ai = true;
                villager.extra.bed = Some([2, 64, 1]);
                game.mobs.push(villager);
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 2.5))
                        .unwrap(),
                );
                assert!(!game.village_centers().is_empty());

                // 1. Production Bad Omen -> Raid Omen insertion order:
                // strength (before bad_omen) ticks; bad_omen inserts raid_omen and stops
                // the pass (vanilla CME), so bad_omen and speed stay un-decremented.
                game.potions.effects = PlayerEffects::default();
                game.potions
                    .effects
                    .add(EffectInstance::new("strength", 50, 0));
                game.potions
                    .effects
                    .add(EffectInstance::new("bad_omen", 100, 1));
                game.potions
                    .effects
                    .add(EffectInstance::new("speed", 50, 0));

                game.tick_mob_effects();
                assert_eq!(game.potions.effects.get("strength").unwrap().duration, 49);
                assert_eq!(game.potions.effects.get("bad_omen").unwrap().duration, 100);
                assert_eq!(game.potions.effects.get("speed").unwrap().duration, 50);
                let raid_omen = game
                    .potions
                    .effects
                    .get("raid_omen")
                    .expect("raid_omen inserted");
                assert_eq!((raid_omen.duration, raid_omen.amplifier), (600, 1));

                // Second tick: strength ticks, bad_omen refreshes raid_omen and is removed,
                // speed and raid_omen both tick down in order.
                game.tick_mob_effects();
                assert_eq!(game.potions.effects.get("strength").unwrap().duration, 48);
                assert!(game.potions.effects.get("bad_omen").is_none());
                assert_eq!(game.potions.effects.get("speed").unwrap().duration, 49);
                let raid_omen = game
                    .potions
                    .effects
                    .get("raid_omen")
                    .expect("raid_omen survives");
                assert_eq!((raid_omen.duration, raid_omen.amplifier), (599, 1));

                // 2. Hidden effect promotion on duration == 1 must not fire MobEffectExpired.
                game.potions.effects = PlayerEffects::default();
                game.potions
                    .effects
                    .add(EffectInstance::new("speed", 100, 0));
                game.potions.effects.add(EffectInstance::new("speed", 1, 1));
                expired_calls.store(0, Ordering::Relaxed);
                game.tick_mob_effects();
                assert_eq!(
                    expired_calls.load(Ordering::Relaxed),
                    0,
                    "promoted hidden effect must not fire MobEffectExpired"
                );
                let speed = game.potions.effects.get("speed").unwrap();
                assert_eq!((speed.amplifier, speed.duration), (0, 99));

                // 3. Steady-state tick_on over boxed hidden chains makes zero heap allocations.
                game.potions.effects = PlayerEffects::default();
                for id in ["speed", "haste", "strength", "jump_boost"] {
                    game.potions.effects.add(EffectInstance::new(id, 400, 0));
                    game.potions.effects.add(EffectInstance::new(id, 200, 1));
                }
                let ((), allocs) = sunrise_threads::allocation_count::measure(|| {
                    PlayerEffects::tick_on(game);
                });
                assert_eq!(
                    allocs, 0,
                    "borrowed PlayerEffects::tick_on must not allocate"
                );
            });
        });
    }
}

/// #4473: the player's own attribute map, which its movement, the hits and
/// the sync read, takes a content mod's declared modifier as vanilla's
/// effects' (`onEffectAdded`, `onEffectUpdated`, `onEffectsRemoved`).
#[cfg(test)]
pub(crate) mod declared_modifier_player_tests {
    use super::*;
    use sunrise_level::brewing::{Category, ModEffect, register_mod_effects};
    use sunrise_sim::motion::PlayerMotion;

    /// `testfx:lucky`, +1 luck per level, in the mod API's registry too (the
    /// removal and the tick ask its events): process-wide, so the test runs
    /// in a fresh child process.
    fn lucky_mod() {
        use mod_api::{self, *};
        struct LuckyMod;
        #[mod_api::__subscriptions]
        impl Hooks for LuckyMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.effect_key("testfx:lucky");
            }
        }
        mod_platform::__private::install(vec![ModEntry::new("testfx", Box::new(LuckyMod))].into());
        assert!(register_mod_effects([ModEffect {
            modifiers: vec![(
                "minecraft:luck".into(),
                "testfx:effect/lucky".into(),
                1.0,
                sunrise_world::attributes::Operation::AddValue,
            )],
            ..ModEffect::new("testfx:lucky", 1, Category::Beneficial, "testfx")
        }]));
    }

    /// The modifier is written on add, rewritten for a higher amplifier and
    /// for the hidden instance taking over in the effect tick, and taken off
    /// by a removal and by the effect running out: the tick's actions carry
    /// the instance's registry index to the writes.
    #[test]
    #[ignore = "daily"]
    fn a_declared_modifier_follows_the_players_effect_in_its_attribute_map() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::potions_server::declared_modifier_player_tests::a_declared_modifier_follows_the_players_effect_in_its_attribute_map",
            "SUNRISE_DECLARED_MODIFIER_PLAYER_CHILD",
            &["lucky"],
        ) {
            return;
        }
        lucky_mod();
        super::super::native_entity_refs::tests::with_game(|game| {
            let lucky = brewing::effect("testfx:lucky").unwrap();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let luck = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:luck")
                    .unwrap()
            };
            assert_eq!(luck(game), 0.0);
            assert!(game.apply_player_effect(EffectInstance::from_def(lucky, -1, 0)));
            assert_eq!(luck(game), 1.0);
            assert!(game.apply_player_effect(EffectInstance::from_def(lucky, -1, 2)));
            assert_eq!(luck(game), 3.0);
            assert!(game.remove_player_effects(None));
            assert_eq!(luck(game), 0.0);
            // Amplifier 1 for 2 ticks over amplifier 0 for 6: the hidden
            // instance takes over, then runs out.
            assert!(game.apply_player_effect(EffectInstance::from_def(lucky, 6, 0)));
            assert!(game.apply_player_effect(EffectInstance::from_def(lucky, 2, 1)));
            assert_eq!(luck(game), 2.0);
            let active = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.potions.effects.get(lucky.id).map(|e| e.amplifier)
            };
            for _ in 0..3 {
                game.tick_mob_effects();
            }
            assert_eq!(active(game), Some(0));
            assert_eq!(luck(game), 1.0);
            for _ in 0..10 {
                game.tick_mob_effects();
            }
            assert_eq!(active(game), None);
            assert_eq!(luck(game), 0.0);
        });
    }
}

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
