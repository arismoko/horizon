//! `LevelRef::explode` through native mod calls: source, interactions, damage type,
//! calculator, effects and unbounded radius.
#![cfg(test)]
use mod_api::{
    EntityRef, ExplodeError, ExplosionCalculator, ExplosionDetonateEvent, ExplosionInteraction,
    ExplosionSettings, ExplosionStartEvent, Flow, HitHost, Hooks, LivingDamagePostEvent,
    LivingDeathEvent, LivingExperienceDropEvent, LootDropsEvent, ModEntry, ServerHost,
};
use std::sync::{Arc, Mutex};
use sunrise_core::{
    geometry::Vec3,
    position::{BlockPos, ChunkPos},
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_level::chunk::Chunk;
use sunrise_sim::{
    mobs::Mob,
    mobs::Who,
    motion::PlayerMotion,
    projectiles::{Projectile, ProjectileKind},
};
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

/// One `explode` call.
struct Blast {
    source: Option<EntityRef>,
    center: [f64; 3],
    radius: f32,
    fire: bool,
    interaction: ExplosionInteraction,
    settings: ExplosionSettings,
}
impl Blast {
    /// A sourceless `Block` blast without fire and with vanilla's settings.
    fn at(center: [f64; 3], radius: f32) -> Self {
        Self {
            source: None,
            center,
            radius,
            fire: false,
            interaction: ExplosionInteraction::Block,
            settings: ExplosionSettings::default(),
        }
    }
}

/// What `explode` answers.
type Answer = Result<(), ExplodeError>;

/// Runs `blast` in the overworld and checks its answer.
fn explode(game: &mut sunrise_gameplay::game::ServerGame<'_>, blast: Blast, expected: Answer) {
    let level = game.level("minecraft:overworld").unwrap();
    assert_eq!(
        level.explode(
            game,
            blast.source,
            blast.center,
            blast.radius,
            blast.fire,
            blast.interaction,
            blast.settings
        ),
        expected
    );
}

/// A server's overworld chunk (0, 0) with a stone floor at y 79.
fn floor(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.build_registries();
    game.load_server_loot();
    game.set_allow_commands(true);
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 79 0 15 79 15 stone"));
}

fn name(game: &sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

/// How many floor blocks are still stone.
fn stone(game: &sunrise_gameplay::game::ServerGame<'_>) -> usize {
    (0..16)
        .flat_map(|x| (0..16).map(move |z| [x, 79, z]))
        .filter(|&p| name(game, p) == "minecraft:stone")
        .count()
}

fn mob(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    id: u32,
    kind: &str,
    at: Vec3,
    health: f32,
) {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut mob = Mob::new(id, kind, at, 0.0);
    mob.persistent = true;
    mob.no_ai = true;
    mob.ground.on_ground = true;
    mob.health = health;
    game.mobs.push(mob);
}

#[derive(Default)]
struct DeathObservations {
    sources: Vec<(u32, String, i32)>,
    loot_players: Vec<Option<u128>>,
    experience: Vec<(u32, Option<u128>, i32)>,
}
struct Deathwatch(Arc<Mutex<DeathObservations>>);
#[mod_api::__subscriptions]
impl Hooks for Deathwatch {
    fn living_death(&self, _: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
        let source = event.source;
        self.0.lock().unwrap().sources.push((
            event.entity,
            source.damage_type.into(),
            source.causing,
        ));
        Flow::Continue
    }

    fn loot_drops(&self, _: &mut dyn ServerHost, event: &mut LootDropsEvent<'_, '_>) {
        if event.context.entity == Some((41, "minecraft:zombie")) {
            self.0
                .lock()
                .unwrap()
                .loot_players
                .push(event.context.player);
        }
    }

    fn living_experience_drop(
        &self,
        _: &mut dyn ServerHost,
        event: &mut LivingExperienceDropEvent,
    ) -> Flow {
        self.0.lock().unwrap().experience.push((
            event.entity,
            event.attacking_player,
            event.original,
        ));
        Flow::Continue
    }
}

/// A loaded arena with the player clear of the blast.
fn credit_arena(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    floor(game);
    assert!(game.run_command("difficulty normal"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 80.0, 14.5)).unwrap());
    bind_local_player(game, 11, 123);
    game.set_mode_id(0);
    game.server.next_entity = 100;
    game.load_ticking_neighbourhood(0, 0);
    game.update_chunk_tickets();
}

/// A blast kills a zombie on one health; check its source and rewards at the hit boundary.
fn kill(
    source: fn(&mut sunrise_gameplay::game::ServerGame<'_>) -> u32,
    (causing, damage_type, credited): (i32, &str, bool),
) {
    crate::entry::install_client_hooks();
    let deaths = Arc::new(Mutex::new(DeathObservations::default()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "explode-deaths",
            Box::new(Deathwatch(deaths.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                credit_arena(game);
                mob(game, 41, "minecraft:zombie", Vec3::new(6.0, 80.0, 4.5), 1.0);
                let id = source(game);
                let source = game.entity(id);
                assert!(source.is_some(), "the source {id} resolves");
                let blast = Blast {
                    source,
                    center: [4.5, 80.0, 4.5],
                    radius: 4.0,
                    fire: false,
                    interaction: ExplosionInteraction::None,
                    settings: ExplosionSettings::default(),
                };
                let mob_kills = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    game.stats.get("custom", "minecraft:mob_kills")
                };
                let expected_kills = mob_kills(game) + if credited { 1 } else { 0 };
                let expected_credit = credited.then(|| game.entity(11).unwrap());
                explode(game, blast, Ok(()));
                let corpse = game.living(game.entity(41).unwrap()).unwrap();
                assert_eq!(corpse.alive(game), Some(false));
                assert_eq!(corpse.kill_credit(game), expected_credit);
                assert_eq!(mob_kills(game), expected_kills);
                // Monster's base reward for an adult zombie with no equipment.
                let xp: i32 = game
                    .orbs
                    .iter()
                    .map(|orb| orb.value * orb.count as i32)
                    .sum();
                assert_eq!(xp, if credited { 5 } else { 0 });

                game.tick_creature_entities().unwrap();
                assert_eq!(mob_kills(game), expected_kills);
                assert_eq!(corpse.kill_credit(game), expected_credit);
                // LivingEntity.tickDeath removes the body at tick 20, before
                // the remembered player would expire on its own.
                for _ in 1..20 {
                    game.tick_creature_entities().unwrap();
                }
                assert!(game.entity(41).is_none());
                assert!(!game.server.last_hurt_by_player.contains_key(&41));
                assert_eq!(mob_kills(game), expected_kills);
            })
        },
    );
    let deaths = deaths.lock().unwrap();
    assert_eq!(deaths.sources, [(41, damage_type.to_owned(), causing)]);
    assert_eq!(deaths.loot_players, [credited.then_some(123)]);
    assert_eq!(
        deaths.experience.as_slice(),
        credited.then_some((41, Some(123), 5)).as_slice()
    );
}

fn kills(
    name: &str,
    source: fn(&mut sunrise_gameplay::game::ServerGame<'_>) -> u32,
    expected: (i32, &'static str, bool),
) {
    if run_fresh_cases(
        &format!("cut_layout::game::server_side::native_explosions_tests::{name}"),
        "SUNRISE_EXPLOSION_DEATH_CHILD",
        &["blast"],
    ) {
        return;
    }
    run_with_game_stack(move || kill(source, expected));
}

#[test]
#[ignore = "daily"]
fn a_player_hit_expires_on_a_living_mob_after_the_last_memory_tick() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_explosions_tests::a_player_hit_expires_on_a_living_mob_after_the_last_memory_tick",
        "SUNRISE_MOB_CREDIT_EXPIRY_CHILD",
        &["expiry"],
    ) {
        return;
    }
    run_with_game_stack(|| {
        with_game(|game| {
            credit_arena(game);
            mob(game, 41, "minecraft:pig", Vec3::new(6.0, 80.0, 4.5), 10.0);
            let living = game.living(game.entity(41).unwrap()).unwrap();
            assert!(game.mob_hits_mob(
                41,
                1.0,
                sunrise_sim::mobs::Damage::player([14.5, 80.0, 14.5])
            ));
            assert_eq!(living.alive(game), Some(true));
            assert_eq!(living.kill_credit(game).map(|killer| killer.id()), Some(11));

            // LivingEntity.resolvePlayerResponsibleForDamage starts the timer at
            // 100; baseTick keeps the reference when it decrements to zero.
            for _ in 0..100 {
                game.tick_creature_entities().unwrap();
            }
            assert_eq!(living.kill_credit(game).map(|killer| killer.id()), Some(11));
            assert!(game.server.last_hurt_by_player.contains_key(&41));
            game.tick_creature_entities().unwrap();
            assert_eq!(living.alive(game), Some(true));
            assert_eq!(living.kill_credit(game), None);
            assert!(!game.server.last_hurt_by_player.contains_key(&41));
        });
    });
}

/// Primed TNT lit by `owner` beside the zombie.
fn tnt(game: &mut sunrise_gameplay::game::ServerGame<'_>, owner: Option<Who>) -> u32 {
    game.add_primed_tnt(BlockPos::new(4, 80, 4), 0.0, 80, owner, None);
    game.tnts.last().unwrap().id
}

/// A summoned entity's id.
fn summoned(game: &mut sunrise_gameplay::game::ServerGame<'_>, kind: &str, nbt: &str) -> u32 {
    let target = game
        .summon_with_nbt(kind, [4.5, 80.5, 4.5], nbt)
        .unwrap()
        .unwrap();
    use sunrise_gameplay_common::game::commands_common::Target;
    match target {
        Target::Stand(id) | Target::Trident(id) | Target::Rocket(id) => id,
        other => panic!("{kind} summoned as {other:?}"),
    }
}

#[test]
#[ignore = "daily"]
fn player_lit_tnt_as_the_source_credits_the_player_with_the_kill() {
    kills(
        "player_lit_tnt_as_the_source_credits_the_player_with_the_kill",
        |game| tnt(game, Some(Who::Player)),
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn unowned_tnt_as_the_source_credits_no_one() {
    kills(
        "unowned_tnt_as_the_source_credits_no_one",
        |game| tnt(game, None),
        (0, "minecraft:explosion", false),
    );
}

#[test]
#[ignore = "daily"]
fn a_player_as_the_source_is_credited_as_a_player() {
    kills(
        "a_player_as_the_source_is_credited_as_a_player",
        |_| 11,
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_living_entity_outside_the_mob_list_causes_its_own_blast() {
    kills(
        "a_living_entity_outside_the_mob_list_causes_its_own_blast",
        |game| summoned(game, "minecraft:armor_stand", "{}"),
        (101, "minecraft:player_explosion", false),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_thrown_trident_credits_the_player() {
    kills(
        "a_players_thrown_trident_credits_the_player",
        |game| {
            let id = summoned(game, "minecraft:trident", "{NoGravity:1b}");
            game.tridents.for_each_mut(|t| t.owner = 11);
            id
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_firework_rocket_credits_the_player() {
    kills(
        "a_players_firework_rocket_credits_the_player",
        |game| {
            let id = summoned(game, "minecraft:firework_rocket", "{LifeTime:100}");
            game.rockets.for_each_mut(|r| r.owner = Some(Who::Player));
            id
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_fishing_bobber_credits_the_player() {
    kills(
        "a_players_fishing_bobber_credits_the_player",
        |game| {
            let hook = sunrise_gameplay_common::game::fishing_common::Hook::remote(
                900,
                Vec3::new(4.5, 81.0, 4.5),
                [1, 2, 3, 4],
            );
            game.install_fishing_hook(hook);
            900
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_mob_blast_breaks_blocks_only_when_its_source_may_grief() {
    run_with_game_stack(|| {
        for griefing in [false, true] {
            with_game(|game| {
                floor(game);
                assert!(game.ui.rules.set(
                    "mob_griefing",
                    sunrise_game_data::gamerules::Value::Bool(griefing),
                ));
                mob(
                    game,
                    42,
                    "minecraft:creeper",
                    Vec3::new(8.5, 80.0, 8.5),
                    20.0,
                );
                let blast = Blast {
                    source: game.entity(42),
                    center: [8.5, 80.0, 8.5],
                    radius: 3.0,
                    fire: false,
                    interaction: ExplosionInteraction::Mob,
                    settings: ExplosionSettings::default(),
                };
                explode(game, blast, Ok(()));
                let (under, floor) = (name(game, [8, 79, 8]), stone(game));
                if griefing {
                    assert_eq!(under, "minecraft:air");
                    assert!(floor < 256, "{floor}");
                } else {
                    assert_eq!((under.as_str(), floor), ("minecraft:stone", 256));
                }
                // The source is spared by its own blast.
                assert_eq!(game.mobs[0].health, 20.0);
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_trigger_blast_presses_a_button_and_breaks_nothing() {
    run_with_game_stack(|| {
        with_game(|game| {
            floor(game);
            assert!(game.run_command("setblock 6 80 4 minecraft:stone_button[face=floor]"));
            let blast = Blast {
                source: None,
                center: [4.5, 80.5, 4.5],
                radius: 3.0,
                fire: false,
                interaction: ExplosionInteraction::Trigger,
                settings: ExplosionSettings::default(),
            };
            explode(game, blast, Ok(()));
            let state = game.world.block(6, 80, 4);
            let powered = game
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("powered").cloned());
            assert_eq!(name(game, [6, 80, 4]), "minecraft:stone_button");
            assert_eq!(powered.as_deref(), Some("true"));
            assert_eq!(stone(game), 256);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_blast_that_keeps_blocks_still_lights_fire() {
    run_with_game_stack(|| {
        with_game(|game| {
            floor(game);
            let blast = Blast {
                source: None,
                center: [8.5, 80.5, 8.5],
                radius: 4.0,
                fire: true,
                interaction: ExplosionInteraction::None,
                settings: ExplosionSettings::default(),
            };
            explode(game, blast, Ok(()));
            let fires = (0..16)
                .flat_map(|x| (0..16).map(move |z| [x, 80, z]))
                .filter(|&p| name(game, p) == "minecraft:fire")
                .count();
            assert!(fires > 0);
            assert_eq!(stone(game), 256);
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_source_explodes_nothing() {
    run_with_game_stack(|| {
        with_game(|game| {
            floor(game);
            mob(
                game,
                43,
                "minecraft:zombie",
                Vec3::new(8.5, 80.0, 9.5),
                20.0,
            );
            let live = game.entity(43).unwrap();
            let stale = EntityRef::new(mod_api::WorldEpoch::new(live.epoch().get() + 1), live.id());
            let nothing = EntityRef::new(live.epoch(), 9999);
            for source in [stale, nothing] {
                let blast = Blast {
                    source: Some(source),
                    center: [8.5, 80.5, 8.5],
                    radius: 4.0,
                    fire: true,
                    interaction: ExplosionInteraction::Block,
                    settings: ExplosionSettings::default(),
                };
                explode(game, blast, Err(ExplodeError::Unavailable));
            }
            assert_eq!(stone(game), 256);
            assert_eq!(game.mobs[0].health, 20.0);
        });
    });
}

/// What a blast's hooks heard: the damage type `explosion_start` names, how
/// often `explosion_detonate` ran, and each `living_damage` (post) with its
/// entity and damage type.
#[derive(Default)]
struct Heard {
    starts: Vec<String>,
    detonates: usize,
    damage: Vec<(i64, String)>,
}
struct Listener(Arc<Mutex<Heard>>);
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn explosion_start(&self, _: &mut dyn ServerHost, event: &mut ExplosionStartEvent<'_>) -> Flow {
        let damage_type = event.explosion.damage_type.to_owned();
        self.0.lock().unwrap().starts.push(damage_type);
        Flow::Continue
    }
    fn explosion_detonate(&self, _: &mut dyn ServerHost, _: &mut ExplosionDetonateEvent<'_>) {
        self.0.lock().unwrap().detonates += 1;
    }
    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        let heard = (i64::from(event.entity), event.source.damage_type.to_owned());
        self.0.lock().unwrap().damage.push(heard);
    }
}

/// Runs `test` on a fresh game with a stone floor, the
/// [`Listener`] hearing it, and hands it what was heard.
fn heard(
    test: impl Fn(&mut sunrise_gameplay::game::ServerGame<'_>, &Arc<Mutex<Heard>>)
    + Send
    + Sync
    + 'static,
) {
    crate::entry::install_client_hooks();
    run_with_game_stack(move || {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let mods = vec![ModEntry::new(
            "explode-heard",
            Box::new(Listener(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                floor(game);
                test(game, &heard);
            })
        });
    });
}

/// The game's damage types plus `test:fuel_tank`, whose death message is
/// `death.attack.fuelTank` and which is in `#minecraft:is_explosion`.
fn register_fuel_tank(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    use serde_json::{Value, json};
    let old = &game.health_resources.damage;
    let mut types: Vec<Value> = old
        .entries()
        .map(|(_, d)| {
            json!({
                "id": d.name, "message_id": d.message_id,
                "exhaustion": format!("{:08x}", d.exhaustion.to_bits()),
                "scaling": d.scaling.name(), "effects": d.effects,
                "death_message_type": d.death_message_type,
            })
        })
        .collect();
    types.push(json!({
        "id": "test:fuel_tank", "message_id": "fuelTank", "exhaustion": "3dcccccd",
        "scaling": "never", "effects": "hurt", "death_message_type": "default",
    }));
    let mut tags = serde_json::Map::new();
    for tag in old.tags() {
        let mut values: Vec<&str> = old
            .entries()
            .filter(|&(id, _)| old.in_tag(id, tag))
            .map(|(_, d)| d.name.as_str())
            .collect();
        if tag == "minecraft:is_explosion" {
            values.push("test:fuel_tank");
        }
        tags.insert(tag.into(), json!({ "values": values }).to_string().into());
    }
    let types = json!({ "data_version": sunrise_world::DATA_VERSION, "types": types });
    game.health_resources.damage =
        sunrise_world::damage_source::Catalog::from_json(&types, &Value::Object(tags)).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_mod_damage_type_names_the_death_message_and_is_what_living_damage_hears() {
    heard(move |game, heard| {
        register_fuel_tank(game);
        mob(
            game,
            44,
            "minecraft:zombie",
            Vec3::new(10.5, 80.0, 8.5),
            20.0,
        );
        let blast = Blast {
            interaction: ExplosionInteraction::None,
            settings: ExplosionSettings {
                damage_type: Some("test:fuel_tank".into()),
                ..Default::default()
            },
            ..Blast::at([8.5, 80.5, 8.5], 4.0)
        };
        explode(game, blast, Ok(()));
        let heard = heard.lock().unwrap();
        assert_eq!(heard.starts, ["test:fuel_tank"]);
        assert_eq!(heard.damage, [(44, "test:fuel_tank".to_owned())]);
        let zombie = game.mobs.remove(0);
        assert!(zombie.health < 20.0, "{}", zombie.health);
        let death = game.mob_death_message(&zombie, None).component();
        assert_eq!(death["translate"], "death.attack.fuelTank");
    });
}

#[test]
#[ignore = "daily"]
fn immune_blocks_survive_and_every_other_cell_resists_nothing() {
    for immune in [Some("minecraft:logs"), None] {
        heard(move |game, _| {
            assert!(game.run_command("setblock 8 80 10 minecraft:oak_log"));
            assert!(game.run_command("setblock 8 80 6 minecraft:obsidian"));
            let blast = Blast {
                settings: ExplosionSettings {
                    calculator: Some(ExplosionCalculator {
                        immune_blocks: immune.map(str::to_owned),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 6.0)
            };
            explode(game, blast, Ok(()));
            let (log, obsidian) = (name(game, [8, 80, 10]), name(game, [8, 80, 6]));
            assert_eq!(name(game, [8, 79, 8]), "minecraft:air", "{immune:?}");
            if immune.is_some() {
                // The tag replaces the rule: obsidian's 1200 counts for nothing.
                assert_eq!(
                    (log.as_str(), obsidian.as_str()),
                    ("minecraft:oak_log", "minecraft:air")
                );
            } else {
                assert_eq!(
                    (log.as_str(), obsidian.as_str()),
                    ("minecraft:air", "minecraft:obsidian")
                );
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_calculator_that_damages_no_entities_leaves_health_while_blocks_break() {
    for damages_entities in [false, true] {
        heard(move |game, heard| {
            mob(
                game,
                45,
                "minecraft:zombie",
                Vec3::new(10.5, 80.0, 8.5),
                20.0,
            );
            let blast = Blast {
                settings: ExplosionSettings {
                    calculator: Some(ExplosionCalculator {
                        damages_entities,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, blast, Ok(()));
            assert!(stone(game) < 256, "blocks break");
            let health = game.mobs[0].health;
            if damages_entities {
                assert!(health < 20.0, "{health}");
            } else {
                assert_eq!(health, 20.0);
                assert!(heard.lock().unwrap().damage.is_empty());
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_knockback_multiplier_of_zero_leaves_velocity_unchanged() {
    for knockback_multiplier in [Some(0.0), None] {
        heard(move |game, _| {
            mob(
                game,
                46,
                "minecraft:zombie",
                Vec3::new(11.5, 80.0, 8.5),
                20.0,
            );
            let blast = Blast {
                interaction: ExplosionInteraction::None,
                settings: ExplosionSettings {
                    calculator: Some(ExplosionCalculator {
                        knockback_multiplier,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, blast, Ok(()));
            let zombie = &game.mobs[0];
            assert!(zombie.health < 20.0, "hurt either way");
            if knockback_multiplier.is_some() {
                assert_eq!(zombie.velocity, Vec3::new(0.0, 0.0, 0.0));
            } else {
                assert!(zombie.velocity.x > 0.0, "{:?}", zombie.velocity);
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_radius_of_48_breaks_a_block_40_blocks_from_the_centre() {
    heard(move |game, _| {
        for x in 1..4 {
            game.world
                .chunks
                .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, 0))
                .unwrap();
        }
        // The ray along (1, 1/15, 1/15) from the centre crosses this block
        // 39.7 blocks out.
        assert!(game.run_command("setblock 48 103 11 minecraft:dirt"));
        explode(game, Blast::at([8.5, 100.5, 8.5], 48.0), Ok(()));
        assert_eq!(name(game, [48, 103, 11]), "minecraft:air");
    });
}

#[test]
#[ignore = "daily"]
fn an_unknown_damage_type_particle_or_tag_explodes_nothing() {
    let refusals = [
        (
            ExplosionSettings {
                damage_type: Some("test:no_such_type".into()),
                ..Default::default()
            },
            ExplodeError::UnknownDamageType,
        ),
        (
            ExplosionSettings {
                large_particle: Some("test:no_such_particle".into()),
                ..Default::default()
            },
            ExplodeError::InvalidParticle,
        ),
        (
            ExplosionSettings {
                block_particles: Some(vec![mod_api::ExplosionBlockParticle {
                    weight: 0,
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ExplodeError::InvalidParticle,
        ),
        (
            ExplosionSettings {
                sound: Some("Not A Sound".into()),
                ..Default::default()
            },
            ExplodeError::UnknownSound,
        ),
        (
            ExplosionSettings {
                calculator: Some(ExplosionCalculator {
                    immune_blocks: Some("test:no_such_tag".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ExplodeError::UnknownBlockTag,
        ),
    ];
    for (settings, refusal) in refusals {
        refused(settings, refusal);
    }
}

/// A blast with `settings` beside a zombie answers `refusal` through both
/// targets: no block breaks, the zombie keeps its health and no hook hears it.
fn refused(settings: ExplosionSettings, refusal: ExplodeError) {
    heard(move |game, heard| {
        mob(
            game,
            47,
            "minecraft:zombie",
            Vec3::new(10.5, 80.0, 8.5),
            20.0,
        );
        let blast = Blast {
            settings: settings.clone(),
            ..Blast::at([8.5, 80.5, 8.5], 4.0)
        };
        explode(game, blast, Err(refusal));
        assert_eq!(stone(game), 256, "{refusal:?}");
        assert_eq!(game.mobs[0].health, 20.0, "{refusal:?}");
        assert!(heard.lock().unwrap().starts.is_empty(), "{refusal:?}");
    });
}

/// Smoke block particles with these weights.
fn weighted(weights: &[u32]) -> ExplosionSettings {
    let list = weights
        .iter()
        .map(|&weight| mod_api::ExplosionBlockParticle {
            weight,
            ..Default::default()
        });
    ExplosionSettings {
        block_particles: Some(list.collect()),
        ..Default::default()
    }
}

#[test]
#[ignore = "daily"]
fn a_block_particle_weight_past_the_packets_int_explodes_nothing() {
    let past = i32::MAX as u32 + 1;
    refused(weighted(&[past]), ExplodeError::InvalidParticle);
}

#[test]
#[ignore = "daily"]
fn block_particle_weights_whose_total_is_past_an_int_explode_nothing() {
    let max = i32::MAX as u32;
    refused(weighted(&[max, 1]), ExplodeError::InvalidParticle);
}

/// A nether portal block at (8, 80, 10) between obsidian on its axis (x) and
/// above and below, which nothing around it changes.
fn portal(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    for pos in ["7 80 10", "9 80 10", "8 81 10", "8 79 10"] {
        assert!(game.run_command(&format!("setblock {pos} minecraft:obsidian")));
    }
    assert!(game.run_command("setblock 8 80 10 minecraft:nether_portal[axis=x]"));
}

#[test]
#[ignore = "daily"]
fn primed_tnt_that_went_through_a_portal_spares_a_portal_unless_a_calculator_is_passed() {
    // (source, calculator, the portal survives)
    let cases = [
        (true, false, true),
        (false, false, false),
        (true, true, false),
    ];
    for (source, calculator, survives) in cases {
        heard(move |game, _| {
            portal(game);
            let source = source.then(|| {
                let id = tnt(game, None);
                game.tnts.last_mut().unwrap().used_portal = true;
                game.entity(id).unwrap()
            });
            let blast = Blast {
                source,
                settings: ExplosionSettings {
                    calculator: calculator.then(ExplosionCalculator::default),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, blast, Ok(()));
            let case = (source.is_some(), calculator);
            assert_eq!(name(game, [8, 79, 8]), "minecraft:air", "{case:?}");
            let expected = if survives {
                "minecraft:nether_portal"
            } else {
                "minecraft:air"
            };
            assert_eq!(name(game, [8, 80, 10]), expected, "{case:?}");
            assert_eq!(name(game, [7, 80, 10]), "minecraft:obsidian", "{case:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_wind_charge_source_brings_its_tag_calculator_unless_a_calculator_is_passed() {
    // (source, calculator, obsidian survives)
    let cases = [
        (true, false, false),
        (false, false, true),
        (true, true, true),
    ];
    for (source, calculator, survives) in cases {
        heard(move |game, _| {
            assert!(game.run_command("setblock 8 80 10 minecraft:obsidian"));
            assert!(game.run_command("setblock 8 80 6 minecraft:bedrock"));
            let source = source.then(|| {
                let charge =
                    Projectile::new(ProjectileKind::WindCharge, Vec3::new(4.5, 80.5, 4.5), None);
                let id = game.add_projectile(charge);
                game.entity(id).unwrap()
            });
            let blast = Blast {
                source,
                settings: ExplosionSettings {
                    calculator: calculator.then(ExplosionCalculator::default),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, blast, Ok(()));
            let case = (source.is_some(), calculator);
            assert_eq!(name(game, [8, 79, 8]), "minecraft:air", "{case:?}");
            // `#blocks_wind_charge_explosions` (barrier, bedrock) stands either way.
            assert_eq!(name(game, [8, 80, 6]), "minecraft:bedrock", "{case:?}");
            let expected = if survives {
                "minecraft:obsidian"
            } else {
                "minecraft:air"
            };
            assert_eq!(name(game, [8, 80, 10]), expected, "{case:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_non_finite_radius_is_refused_and_one_of_zero_finds_no_blocks() {
    heard(move |game, heard| {
        for radius in [f32::NAN, f32::INFINITY] {
            let blast = Blast::at([8.5, 80.5, 8.5], radius);
            explode(game, blast, Err(ExplodeError::InvalidBlast));
        }
        assert!(heard.lock().unwrap().starts.is_empty());
        // As in vanilla: the start hook, no blocks, no detonate.
        explode(game, Blast::at([8.5, 79.5, 8.5], 0.0), Ok(()));
        assert_eq!(stone(game), 256);
        let heard = heard.lock().unwrap();
        assert_eq!((heard.starts.len(), heard.detonates), (1, 0));
    });
}

/// Runs with a still zombie and a placed server player on either side of a [`wind_blast`].
fn heard_with_targets(
    flying: bool,
    test: impl Fn(&mut sunrise_gameplay::game::ServerGame<'_>, &Arc<Mutex<Heard>>)
    + Send
    + Sync
    + 'static,
) {
    use sunrise_command::args::GameMode;
    use sunrise_gameplay::game::native_guest_turn_tests::{add_fixture_guests, open_fixture_hub};

    heard(move |game, events| {
        const PLAYER: u32 = 12;
        assert!(game.run_command("difficulty normal"));
        open_fixture_hub(game);
        add_fixture_guests(game);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(PLAYER, |guest| {
                // Keep the ordinary blast in range but nonlethal for the player.
                guest.position = [3.5, 80.0, 8.5];
                guest.loaded = true;
                guest.load_timeout = 0;
                if flying {
                    let mode = GameMode::Creative.id();
                    guest.change_game_mode(mode);
                    guest.abilities.update_for_mode(mode);
                    guest.abilities.flying = true;
                }
            })
            .unwrap();
        game.server.next_entity = 100;
        mob(
            game,
            48,
            "minecraft:zombie",
            Vec3::new(11.5, 80.0, 8.5),
            20.0,
        );
        game.as_guest(PLAYER, |game| test(game, events)).unwrap();
    });
}

/// A wind charge of `kind` above the blast, as its source.
fn wind_charge(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    kind: ProjectileKind,
) -> Option<EntityRef> {
    let charge = Projectile::new(kind, Vec3::new(8.5, 86.5, 8.5), None);
    let id = game.add_projectile(charge);
    Some(game.entity(id).expect("wind charge must be resident"))
}

/// A blast at (8.5, 80.5, 8.5), radius 4, that keeps its blocks.
fn wind_blast(source: Option<EntityRef>, calculator: Option<ExplosionCalculator>) -> Blast {
    Blast {
        source,
        interaction: ExplosionInteraction::None,
        settings: ExplosionSettings {
            calculator,
            ..Default::default()
        },
        ..Blast::at([8.5, 80.5, 8.5], 4.0)
    }
}

/// A calculator that pushes by 1 and damages no one.
fn unhurt() -> Option<ExplosionCalculator> {
    Some(ExplosionCalculator {
        damages_entities: false,
        ..Default::default()
    })
}

/// The zombie's and the player's velocity, both stilled again.
fn pushes(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> (Vec3, Vec3) {
    let still = Vec3::new(0.0, 0.0, 0.0);
    let zombie = std::mem::replace(&mut game.mobs[0].velocity, still);
    let player = &mut game.player.as_mut().unwrap().velocity;
    (zombie, std::mem::replace(player, still))
}

#[test]
#[ignore = "daily"]
fn a_wind_charge_source_damages_neither_a_mob_nor_a_player() {
    for kind in [ProjectileKind::WindCharge, ProjectileKind::BreezeWindCharge] {
        heard_with_targets(false, move |game, heard| {
            let source = wind_charge(game, kind.clone());
            explode(game, wind_blast(source, None), Ok(()));
            let player = game.player.as_ref().unwrap().health.health();
            assert_eq!((game.mobs[0].health, player), (20.0, 20.0), "{kind:?}");
            assert!(heard.lock().unwrap().damage.is_empty(), "{kind:?}");
            // Knockback still applies.
            let (zombie, player) = pushes(game);
            assert!(zombie.x > 0.0 && player.x < 0.0, "{kind:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_wind_charge_source_pushes_by_1_22_for_a_players_and_1_for_a_breezes() {
    let cases = [
        (ProjectileKind::WindCharge, 1.22_f32),
        (ProjectileKind::BreezeWindCharge, 1.0),
    ];
    for (kind, multiplier) in cases {
        heard_with_targets(false, move |game, _| {
            explode(game, wind_blast(None, unhurt()), Ok(()));
            let plain = pushes(game);
            let source = wind_charge(game, kind.clone());
            explode(game, wind_blast(source, None), Ok(()));
            let wind = pushes(game);
            let m = f64::from(multiplier);
            for (wind, plain) in [(wind.0, plain.0), (wind.1, plain.1)] {
                assert!(plain.x.abs() > 0.1, "{kind:?}: {plain:?}");
                let scaled = Vec3::new(plain.x * m, plain.y * m, plain.z * m);
                let off = (wind.x - scaled.x).abs()
                    + (wind.y - scaled.y).abs()
                    + (wind.z - scaled.z).abs();
                assert!(off < 1.0e-9, "{kind:?}: {wind:?} vs {scaled:?}");
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_flying_player_is_not_moved_by_a_wind_charge_source() {
    for kind in [ProjectileKind::WindCharge, ProjectileKind::BreezeWindCharge] {
        heard_with_targets(true, move |game, _| {
            let source = wind_charge(game, kind.clone());
            explode(game, wind_blast(source, None), Ok(()));
            let (zombie, player) = pushes(game);
            assert!(zombie.x > 0.0, "{kind:?}: {zombie:?}");
            assert_eq!(player, Vec3::new(0.0, 0.0, 0.0), "{kind:?}");
            // The default calculator pushes a flying player like any other.
            explode(game, wind_blast(None, None), Ok(()));
            let (_, player) = pushes(game);
            assert!(player.x < 0.0, "{kind:?}: {player:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_passed_calculator_replaces_a_wind_charges_entity_rules() {
    heard_with_targets(false, |game, _| {
        explode(game, wind_blast(None, unhurt()), Ok(()));
        let (plain_zombie, plain_player) = pushes(game);
        assert!(plain_zombie.x > 0.0, "{plain_zombie:?}");
        assert!(plain_player.x < 0.0, "{plain_player:?}");
        assert_eq!(game.mobs[0].health, 20.0);
        assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
        let source = wind_charge(game, ProjectileKind::WindCharge);
        let calculator = Some(ExplosionCalculator::default());
        explode(game, wind_blast(source, calculator), Ok(()));
        // The explicit calculator damages both targets and uses the ordinary push multiplier.
        let player_health = game.player.as_ref().unwrap().health.health();
        assert!(game.mobs[0].health < 20.0);
        assert!(player_health < 20.0, "{player_health}");
        let (zombie, player) = pushes(game);
        assert_eq!(zombie, plain_zombie, "zombie knockback");
        assert_eq!(player, plain_player, "player knockback");
    });
}

#[test]
#[ignore = "daily"]
fn unloaded_centre_and_far_coordinates_still_explode_without_loading_chunks() {
    heard(move |game, heard| {
        // Centre in unloaded chunk (-1, 0), beside chunk (0, 0)'s stone floor.
        explode(game, Blast::at([-1.5, 79.5, 8.5], 4.0), Ok(()));
        assert!(!game.world.chunks.contains_key(&(-1, 0)), "never loaded");
        assert!(
            stone(game) < 256,
            "rays from an unloaded centre still reach adjacent loaded blocks"
        );
        // Centre past 30,000,000 is finite and accepted like ServerLevel.explode.
        explode(game, Blast::at([4.0e7, 80.5, 8.5], 4.0), Ok(()));
        let heard = heard.lock().unwrap();
        assert_eq!((heard.starts.len(), heard.detonates), (2, 2));
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
