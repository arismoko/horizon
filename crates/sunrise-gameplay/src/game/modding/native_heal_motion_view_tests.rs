//! A heal or movement listener reads the entity it is about through the view the hook lends
//! (`LivingHealEvent.getEntity`; the entity whose `getJumpPower`, `getSpeed` and
//! `getFlyingSpeed` the game computes), from its live body: an effect added or removed earlier in
//! the same tick is what it reads, and Jump Boost, which vanilla's `getJumpBoostPower` reads with
//! `hasEffect` at the call, agrees with it.
use super::native_entity_refs::tests::{bind_local_player, run_fresh_cases, with_game};
use super::native_living_refs::tests::summon_mob;
use crate::game::ServerGame;
use mod_api::{
    ActiveEffect, EffectKey, Flow, Holder, Hooks, LivingFlyingSpeedEvent, LivingHealEvent,
    LivingJumpPowerEvent, LivingSpeedEvent, LivingView,
};
use std::sync::{Arc, Mutex};
use sunrise_core::{game_mode::GameMode, geometry::Vec3};
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_sim::motion::{MotionWorld, PlayerMotion};
use sunrise_world::living_air::LivingAirWorld;

fn effect(id: &str) -> EffectKey {
    mod_platform::__private::part_tests::effect_key(id)
}

/// What a listener read of the entity its hook lends.
#[derive(Clone, Debug, PartialEq)]
struct Read {
    id: u32,
    kind: Option<mod_api::EntityTypeKey>,
    health: Option<f32>,
    max_health: Option<f32>,
    /// The Jump Boost and Speed effects' amplifier and ticks left.
    jump_boost: Option<(i32, i32)>,
    speed: Option<(i32, i32)>,
    weakened: bool,
    effects: Option<usize>,
    holder: Option<Holder>,
}

fn view_read(view: &LivingView<'_>) -> Read {
    let pair = |e: ActiveEffect| (e.amplifier, e.duration);
    Read {
        id: view.id(),
        kind: view.kind(),
        health: view.health(),
        max_health: view.max_health(),
        jump_boost: view.effect(effect("minecraft:jump_boost")).map(pair),
        speed: view.effect(effect("minecraft:speed")).map(pair),
        weakened: view.potion_affected(effect("minecraft:weakness")),
        effects: view.effects().map(|list| list.len()),
        holder: view.holder(),
    }
}

/// One hook call: which hook, the event's entity, the value it carried in, and the view's reads.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    hook: &'static str,
    entity: u32,
    value: f32,
    read: Read,
}

/// Records each call; a jump of an entity that is weakened jumps half as high, the custom
/// modifier a mod would combine with vanilla's Jump Boost.
struct Probe(Arc<Mutex<Vec<Seen>>>);

impl Probe {
    fn seen(&self, hook: &'static str, entity: u32, value: f32, view: &LivingView<'_>) {
        self.0.lock().unwrap().push(Seen {
            hook,
            entity,
            value,
            read: view_read(view),
        });
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_heal(&self, _: &dyn mod_api::LoopContext, event: &mut LivingHealEvent<'_>) -> Flow {
        self.seen("heal", event.entity, event.amount, event.healed);
        Flow::Continue
    }
    fn living_jump_power(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut LivingJumpPowerEvent<'_>,
    ) {
        self.seen("jump_power", event.entity, event.power, event.living);
        if event.living.potion_affected(effect("minecraft:weakness")) {
            event.power *= 0.5;
        }
    }
    fn living_speed(&self, _: &dyn mod_api::LoopContext, event: &mut LivingSpeedEvent<'_>) {
        self.seen("speed", event.entity, event.speed, event.living);
    }
    fn living_flying_speed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut LivingFlyingSpeedEvent<'_>,
    ) {
        self.seen("flying_speed", event.entity, event.speed, event.living);
    }
}

fn add(game: &mut ServerGame<'_>, id: u32, key: &str, duration: i32, amplifier: i32) {
    let living = game.living(game.entity(id).unwrap()).unwrap();
    let effect = ActiveEffect::new(effect(key), duration, amplifier, false, true, true);
    assert!(game.living_add_effect(living, &effect));
}

fn remove(game: &mut ServerGame<'_>, id: u32, key: &str) {
    let living = game.living(game.entity(id).unwrap()).unwrap();
    assert!(game.living_remove_effect(living, effect(key)));
}

/// The player's jump power, speed and flying speed, computed by its motion world as its tick
/// does, through the three hooks.
fn player_motion_values(game: &mut ServerGame<'_>) -> (f32, f32, f32) {
    let Some(player) = game.common.player.as_mut() else {
        panic!("a player in place");
    };
    let side = server_game_side!(game);
    let mut w = MotionWorld {
        side: &side,
        entities: &[],
        player,
        scene: &mut game.common.world,
        assets: &game.common.pack.assets,
        collision_binding: game.common.collision,
        resources: game.common.motion,
        health: &game.common.health_resources,
        tag_items: Some(server_world_items!(game)),
        loop_context: &game.common.loop_context,
        damage_hooks: &game.common.damage_hooks,
        block_registries: Some(&game.common.pack.block_registries),
    };
    let jump = sunrise_world::living_motion::jump_power(&mut w, 1.0).unwrap();
    let speed = LivingAirWorld::speed(&mut w).unwrap();
    let flying = LivingAirWorld::flying_speed(&mut w).unwrap();
    (jump, speed, flying)
}

fn with_probe(test: impl FnOnce(&mut ServerGame<'_>, &Mutex<Vec<Seen>>)) {
    sunrise_gameplay_common::game::native_hooks::install_client_hooks();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // The column the fixture's mobs stand in, so their handles resolve.
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            bind_local_player(game, 7, 7);
            game.set_game_mode(GameMode::Survival);
            test(game, &seen);
        });
    });
}

fn take(seen: &Mutex<Vec<Seen>>) -> Vec<Seen> {
    std::mem::take(&mut *seen.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn a_heal_or_movement_listener_reads_the_entity_live_through_its_view() {
    const CASE: &str = "SUNRISE_TEST_HEAL_MOTION_VIEWS";
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_heal_or_movement_listener_reads_the_entity_live_through_its_view"
        )
        .split_once("::")
        .unwrap()
        .1,
        CASE,
        &["heal", "motion"],
    ) {
        return;
    }
    match std::env::var(CASE).as_deref() {
        Ok("heal") => heals(),
        Ok("motion") => motion_values(),
        other => panic!("unknown case {other:?}"),
    }
}

/// Each heal site lends the healed entity: the player through `living.heal` (an instant
/// effect's path), peaceful regeneration and the food tick, and a mob through `living.heal`.
fn heals() {
    with_probe(|game, seen| {
        let key = mod_api::__private::resolved_entity_type_key;
        let player_kind = Some(key("minecraft:player").expect("player"));
        let zombie = summon_mob(game, "minecraft:zombie", "{}");
        let zombie_kind = Some(key("minecraft:zombie").expect("zombie"));
        let mob = game.living(game.entity(zombie).unwrap()).unwrap();
        assert!(mob.set_health(game, 12.0));
        let player = game.living(game.entity(7).unwrap()).unwrap();
        assert!(player.set_health(game, 10.0));

        // Added just before the heal, in the same tick: the listener reads it.
        add(game, 7, "minecraft:speed", 400, 1);
        assert!(game.living_heal(player, 2.0));
        let player_read = |health, speed, effects| Read {
            id: 7,
            kind: player_kind,
            health: Some(health),
            max_health: Some(20.0),
            jump_boost: None,
            speed,
            weakened: false,
            effects: Some(effects),
            holder: Some(Holder::Player(7)),
        };
        assert_eq!(
            take(seen),
            [Seen {
                hook: "heal",
                entity: 7,
                value: 2.0,
                read: player_read(10.0, Some((1, 400)), 1),
            }]
        );
        assert_eq!(game.player.as_ref().unwrap().health.health(), 12.0);

        // Removed just before the next heal: the listener reads none.
        remove(game, 7, "minecraft:speed");
        assert!(game.living_heal(player, 1.0));
        assert_eq!(
            take(seen),
            [Seen {
                hook: "heal",
                entity: 7,
                value: 1.0,
                read: player_read(12.0, None, 0),
            }]
        );

        // Peaceful regeneration (`Player.aiStep`): one health every 20 ticks.
        let mut rules = game.world.health_rules;
        rules.difficulty = sunrise_world::food::Difficulty::Peaceful;
        rules.natural_regeneration = true;
        add(game, 7, "minecraft:speed", 300, 0);
        let p = game.common.player.as_mut().unwrap();
        p.health.tick_count = 20;
        p.peaceful_regeneration(
            &game.common.loop_context,
            &|| crate::game::entity_server::hit_reads_of!(game),
            rules,
        )
        .unwrap();
        assert_eq!(
            take(seen),
            [Seen {
                hook: "heal",
                entity: 7,
                value: 1.0,
                read: player_read(13.0, Some((0, 300)), 1),
            }]
        );

        // The food tick (`FoodData.tick`): full food and 5 saturation heal 5/6 on the tenth tick.
        rules.difficulty = sunrise_world::food::Difficulty::Normal;
        let p = game.player.as_mut().unwrap();
        p.health.food.food = 20;
        p.health.food.saturation = 5.0;
        for _ in 0..10 {
            let p = game.common.player.as_mut().unwrap();
            p.food_tick(
                &game.common.loop_context,
                crate::game::entity_server::damage_level_of!(
                    game,
                    &sunrise_sim::health::NO_ATTACK,
                    no_push,
                    None,
                    sunrise_gameplay_common::acting_equipment!(game.common)
                ),
                &game.common.health_resources,
                rules,
            )
            .unwrap();
        }
        assert_eq!(
            take(seen),
            [Seen {
                hook: "heal",
                entity: 7,
                value: 5.0 / 6.0,
                read: player_read(14.0, Some((0, 300)), 1),
            }]
        );

        // A mob: its own effects, health and type.
        add(game, zombie, "minecraft:jump_boost", 200, 2);
        assert!(game.living_heal(mob, 3.0));
        assert_eq!(
            take(seen),
            [Seen {
                hook: "heal",
                entity: zombie,
                value: 3.0,
                read: Read {
                    id: zombie,
                    kind: zombie_kind,
                    health: Some(12.0),
                    max_health: Some(20.0),
                    jump_boost: Some((2, 200)),
                    speed: None,
                    weakened: false,
                    effects: Some(1),
                    holder: Some(Holder::Entity(zombie)),
                },
            }]
        );
    });
}

/// The player's and a mob's jump power with Jump Boost and a listener's own modifier, both
/// changed in the same tick: the power and the view agree at each call.
fn motion_values() {
    with_probe(|game, seen| {
        // Jump Boost II and Weakness added in this tick: vanilla's 0.42 jump strength plus
        // 0.1 × 2, then halved by the listener.
        add(game, 7, "minecraft:jump_boost", 400, 1);
        add(game, 7, "minecraft:weakness", 400, 0);
        let (jump, speed, flying) = player_motion_values(game);
        assert_eq!((jump, speed, flying), (0.31, 0.1, 0.02));
        let calls = take(seen);
        let hooks: Vec<_> = calls.iter().map(|c| (c.hook, c.entity, c.value)).collect();
        assert_eq!(
            hooks,
            [
                ("jump_power", 7, 0.62),
                ("speed", 7, 0.1),
                ("flying_speed", 7, 0.02)
            ]
        );
        for call in &calls {
            assert_eq!(
                (
                    call.read.jump_boost,
                    call.read.weakened,
                    call.read.health,
                    call.read.holder
                ),
                (Some((1, 400)), true, Some(20.0), Some(Holder::Player(7))),
                "{}",
                call.hook
            );
        }

        // Weakness removed and Jump Boost raised in the same tick: no halving, 0.1 × 4.
        remove(game, 7, "minecraft:weakness");
        add(game, 7, "minecraft:jump_boost", 100, 3);
        let (jump, _, _) = player_motion_values(game);
        assert_eq!(jump, 0.82);
        let calls = take(seen);
        assert_eq!((calls[0].hook, calls[0].value), ("jump_power", 0.82));
        assert_eq!(
            (calls[0].read.jump_boost, calls[0].read.weakened),
            (Some((3, 100)), false)
        );

        // A mob: its own Jump Boost and the listener's halving, read the same way.
        let zombie = summon_mob(game, "minecraft:zombie", "{}");
        add(game, zombie, "minecraft:jump_boost", 200, 0);
        add(game, zombie, "minecraft:weakness", 200, 0);
        let jump = {
            let w = game.mob_world_base(&[]);
            let m = game.mobs.iter().find(|m| m.id == zombie).unwrap();
            m.jump_power(&w)
        };
        assert_eq!(jump, 0.26);
        let calls = take(seen);
        assert_eq!(
            (calls[0].hook, calls[0].entity, calls[0].value),
            ("jump_power", zombie, 0.52)
        );
        assert_eq!(
            (
                calls[0].read.jump_boost,
                calls[0].read.weakened,
                calls[0].read.holder
            ),
            (Some((0, 200)), true, Some(Holder::Entity(zombie)))
        );
    });
}
