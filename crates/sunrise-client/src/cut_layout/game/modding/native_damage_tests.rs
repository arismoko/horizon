//! `living_damage_pre` and `living_damage_post` heard by a trait mod, through the game's own
//! hooks: the damage container each hit fills, the hit's level host, and the mob and dragon
//! hits that fire them.
use super::native_entity_refs::tests::{bind_local_player, with_game};
use mod_api::{
    self, HitHost, Hooks, HurtSource, LivingDamagePostEvent, LivingDamagePreEvent,
    LivingIncomingDamageEvent, LivingShieldBlockEvent, Reduction,
};
use std::sync::{Arc, Mutex};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Damage, DamageKind, Mob};
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay_common::game::dragon_common::Dragon,
    sunrise_gameplay_common::game::dragon_common::SPECIAL,
};

const SLEEPING_BED: [i32; 3] = [4, 100, 4];

struct ObserveBedDuringDamage(Arc<Mutex<Vec<&'static str>>>);

impl ObserveBedDuringDamage {
    fn observe(&self, level: &dyn HitHost, phase: &'static str) {
        let state = level.block_state(SLEEPING_BED).unwrap();
        assert_eq!(level.state_block(state), Some("minecraft:red_bed"));
        assert_eq!(
            level.state_with_property(state, "occupied", "false"),
            Some(state)
        );
        self.0.lock().unwrap().push(phase);
    }
}

#[mod_api::__subscriptions]
impl Hooks for ObserveBedDuringDamage {
    fn living_damage_pre(&self, level: &mut dyn HitHost, _: &mut LivingDamagePreEvent<'_>) {
        self.observe(level, "pre");
    }

    fn living_damage_post(&self, level: &mut dyn HitHost, _: &mut LivingDamagePostEvent<'_>) {
        self.observe(level, "post");
    }
}

#[test]
#[ignore = "daily"]
fn brainless_villager_wakes_before_damage_listeners() {
    use super::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, run_fresh_cases};
    use sunrise_core::{nbt::Tag, position::ChunkPos};
    use sunrise_sim::mobs::{villager, villager_brain};

    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::brainless_villager_wakes_before_damage_listeners",
        "SUNRISE_TEST_BRAINLESS_VILLAGER_DAMAGE",
        &["brainless"],
    ) {
        return;
    }

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "observe_bed",
        Box::new(ObserveBedDuringDamage(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            assert!(game.run_command("fill 0 99 0 7 99 7 stone"));
            assert!(game.run_command(
                "setblock 4 100 4 red_bed[part=head,facing=north,occupied=false] strict"
            ));
            assert!(game.run_command(
                "setblock 4 100 5 red_bed[part=foot,facing=north,occupied=false] strict"
            ));
            let id = game.take_entity_id();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(id, kind, Vec3::new(3.5, 100.0, 4.5), 0.0));
            let mob = game.mobs.last_mut().unwrap();
            let job = [2, 100, 4];
            let meeting = [3, 100, 4];
            mob.extra.bed = Some(SLEEPING_BED);
            mob.extra.home = Some(job);
            mob.extra.meeting = Some(meeting);
            let epoch = game.native_lifecycle.epoch().unwrap();
            let living = mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, id));
            game.world.game_time = 37;
            assert!(game.living_start_sleeping(living, SLEEPING_BED));
            let ext = villager::ext(game.mob_object(id).unwrap()).unwrap();
            assert!(ext.brain.is_none() && ext.saved_memories.is_none());
            assert_eq!(ext.last_slept, Some(37));

            game.world.game_time = 89;
            assert!(game.run_command(
                "damage @e[type=villager,limit=1] 2 minecraft:generic at 2.5 100 4.5"
            ));
            assert_eq!(&*heard.lock().unwrap(), &["pre", "post"]);
            let mob = game.mobs.last_mut().unwrap();
            assert_eq!(mob.health, 18.0);
            assert!(!villager::sleeping(mob));
            assert!(villager::ext(mob).unwrap().brain.is_none());

            villager_brain::init(mob, 89, 14000, i64::from(id));
            let mut written = Vec::new();
            villager::write_memories(mob, &mut written);
            let memories = Tag::compound(written);
            let memory = |key| memories.get(key).and_then(|tag| tag.get("value"));
            assert_eq!(memory("minecraft:last_slept"), Some(&Tag::Long(37)));
            assert_eq!(memory("minecraft:last_woken"), Some(&Tag::Long(89)));
            for (key, pos) in [
                ("minecraft:home", SLEEPING_BED),
                ("minecraft:job_site", job),
                ("minecraft:meeting_point", meeting),
            ] {
                assert_eq!(
                    memory(key).and_then(|tag| tag.get("pos")),
                    Some(&Tag::IntArray(pos.to_vec()))
                );
            }
        });
    });
}

/// What the probe heard: Pre's entity, original damage, new damage and armor's reduction, and
/// Post's health damage, then its entity, health damage, absorption reduction and blocked damage.
#[derive(Debug, Default, PartialEq)]
struct Heard {
    pre: Vec<(i32, f32, f32, f32)>,
    post: Vec<(i32, f32)>,
    post_values: Vec<(i32, f32, f32, f32)>,
    sources: Vec<HitSource>,
    weapons: Vec<(Option<String>, Option<String>)>,
    positions: Vec<(&'static str, Option<[f64; 3]>)>,
    side_effects: Vec<(i32, bool)>,
}

#[derive(Debug, PartialEq)]
struct HitSource {
    phase: &'static str,
    entity: i32,
    causing: i32,
    direct: i32,
    direct_type: Option<String>,
}

impl Heard {
    fn source(&mut self, phase: &'static str, entity: i32, source: &HurtSource<'_>) {
        self.positions.push((phase, source.position));
        self.sources.push(HitSource {
            phase,
            entity,
            causing: source.causing,
            direct: source.direct,
            direct_type: source.direct_type.map(str::to_owned),
        });
    }
}

/// The test mod: 225001's incoming hit becomes 4, Pre sets 225003's and 225008's damage to 2,
/// and draws once from the level's random for 225010.
struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .lock()
            .unwrap()
            .source("incoming", event.entity, event.source);
        let item_id = |stack: Option<&mod_api::ItemStackView<'_>>| {
            stack.and_then(|s| mod_api::__private::item_key_id(s.item()).map(str::to_owned))
        };
        let direct = item_id(event.weapon);
        let causing = item_id(event.causing_weapon);
        if causing.as_deref() == Some("minecraft:iron_sword") {
            event.container.set_new_damage(1.0).unwrap();
        }
        self.0.lock().unwrap().weapons.push((direct, causing));
        if event.entity == 225020 && event.source.position == Some([0.0, 0.0, -2.0]) {
            event.container.set_new_damage(4.0).unwrap();
        }
        if event.entity == 225001 {
            event.container.set_new_damage(4.0).unwrap();
        }
        if event.entity == 225033 {
            event
                .container
                .add_reduction_modifier(Reduction::Armor, |_, _| -1.0e38);
        }
        mod_api::Flow::Continue
    }
    fn living_shield_block(&self, _: &mut dyn HitHost, event: &mut LivingShieldBlockEvent<'_>) {
        if event.entity == 227002 {
            event.block.blocked = true;
            event.block.blocked_damage = event.damage;
        }
    }
    fn living_damage_pre(&self, level: &mut dyn HitHost, event: &mut LivingDamagePreEvent<'_>) {
        let container = &mut event.container;
        if matches!(event.entity, 225003 | 225008) {
            container.set_new_damage(2.0).unwrap();
        }
        if event.entity == 225010 {
            level.with_random(|random| random.next_int()).unwrap();
        }
        if event.entity == 225033 {
            assert_eq!(container.new_damage(), 1.0e38);
            assert_eq!(container.reduction(Reduction::MobEffects), 0.0);
            container.set_new_damage(1.0).unwrap();
        }
        let heard = (
            event.entity,
            container.original(),
            container.new_damage(),
            container.reduction(Reduction::Armor),
        );
        let mut log = self.0.lock().unwrap();
        log.pre.push(heard);
        log.source("pre", event.entity, event.source);
    }
    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        let heard = (event.entity, event.container.health_damage());
        let mut log = self.0.lock().unwrap();
        log.post.push(heard);
        log.post_values.push((
            event.entity,
            event.container.health_damage(),
            event.container.reduction(Reduction::Absorption),
            event.container.blocked(),
        ));
        log.source("post", event.entity, event.source);
        log.side_effects
            .push((event.entity, event.container.should_cause_side_effects()));
    }
}

/// Runs `test` in a game with the probe installed, and returns what it heard.
fn heard(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) -> Heard {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(test));
    std::mem::take(&mut *log.lock().unwrap())
}

fn mob(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32, kind: &str) -> Mob {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    Mob::new(id, kind, Vec3::ZERO, 0.0)
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn the_original_damage_is_the_hit_before_the_incoming_hook() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225001, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            10.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 6.0);
    });
    assert_eq!(heard.pre, [(225001, 10.0, 4.0, 0.0)]);
    assert_eq!(heard.post, [(225001, 4.0)]);
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn pre_sets_health_lost_but_cooldown_compares_the_outer_hit() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225003, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            10.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 8.0);
        assert_eq!(pig.last_hurt, 10.0);
        assert!(!pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            5.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 8.0);
    });
    assert_eq!(heard.pre, [(225003, 10.0, 2.0, 0.0)]);
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn pre_fires_for_a_hit_its_reductions_took_to_zero() {
    let heard = heard(|game| {
        let mut witch = mob(game, 225008, "minecraft:witch");
        // Witch.getDamageAfterMagicAbsorb: none from its own doing.
        let own = Damage::mob(225008, [2.0, 0.0, 2.0]);
        assert!(witch.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            6.0,
            own
        ));
        assert_eq!(witch.health, 24.0);
    });
    assert_eq!(heard.pre, [(225008, 6.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn pre_draws_from_the_level_random_of_the_hit() {
    use sunrise_core::random::RandomSource as _;

    heard(|game| {
        let mut expected = game.world.random.clone();
        expected.next_int();
        let mut pig = mob(game, 225010, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            1.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(game.world.random.next_int(), expected.next_int());
    });
}

#[test]
#[ignore = "daily"]
fn a_hit_on_the_dragon_fires_pre_and_post() {
    let heard = heard(|game| {
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225009;
        game.dragon.replace(dragon);
        let _ = game.hit_special(SPECIAL, 6.0, true, DamageKind::Generic, None);
        assert_eq!(game.dragon.as_ref().unwrap().health, 194.0);
    });
    assert_eq!(heard.pre, [(225009, 6.0, 6.0, 0.0)]);
    assert_eq!(heard.post, [(225009, 6.0)]);
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn mob_and_dragon_post_records_fresh_hit_decision() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225014, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            2.0,
            source
        ));
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            4.0,
            source
        ));
        assert!(!pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            3.0,
            source
        ));
        assert_eq!(pig.health, 6.0);
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225015;
        game.dragon.replace(dragon);
        for amount in [2.0, 4.0, 3.0] {
            let _ = game.hit_special(SPECIAL, amount, true, DamageKind::Generic, None);
        }
        assert_eq!(game.dragon.as_ref().unwrap().health, 196.0);
    });
    assert_eq!(
        heard.post,
        [(225014, 2.0), (225014, 2.0), (225015, 2.0), (225015, 2.0)]
    );
    assert_eq!(
        heard.side_effects,
        [
            (225014, true),
            (225014, false),
            (225015, true),
            (225015, false)
        ]
    );
}

fn survival_player(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    bind_local_player(game, id, u128::from(id));
    game.set_game_mode(GameMode::Survival);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_that_absorption_took_whole() {
    let heard = heard(|game| {
        survival_player(game, 227001);
        // Absorption is cut to `max_absorption`, 0 without a modifier.
        let player = game.player.as_mut().unwrap();
        player
            .set_modifier(
                "minecraft:max_absorption",
                "test:absorption",
                10.0,
                "add_value",
                true,
            )
            .unwrap();
        player.set_health(2.0, true).unwrap();
        assert!(game.hurt_player_from(
            2.0,
            None,
            "minecraft:generic",
            sunrise_gameplay_common::game::creatures::KnownCause::None,
            false
        ));
        let health = &game.player.as_ref().unwrap().health;
        assert_eq!((health.health(), health.absorption()), (20.0, 0.0));
    });
    assert_eq!(heard.post_values, [(227001, 0.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_a_shield_blocked_whole() {
    let heard = heard(|game| {
        survival_player(game, 227002);
        game.hurt_player_from(
            2.0,
            None,
            "minecraft:generic",
            sunrise_gameplay_common::game::creatures::KnownCause::None,
            false,
        );
        assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
    });
    assert_eq!(heard.post_values, [(227002, 0.0, 0.0, 2.0)]);
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn a_hit_inside_the_cooldown_no_stronger_than_the_last_fires_no_post() {
    let heard = heard(|game| {
        // Pre sets the health damage to 2 without changing `lastHurt`.
        let mut pig = mob(game, 225003, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            4.0,
            source
        ));
        // Inside the cooldown and no stronger than the last hit: refused, no Post.
        assert!(!pig.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            2.0,
            source
        ));
    });
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_once_for_a_killing_blow_on_a_mob() {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let pig = mob(game, 225016, "minecraft:pig");
            game.mobs.push(pig);
            let victim = game.entity(225016).unwrap();
            assert_eq!(
                victim.hurt(game, "minecraft:generic", 100.0, None, None),
                Some(true),
            );
            assert_eq!(game.mobs[0].health, 0.0);
            assert_eq!(log.lock().unwrap().post, [(225016, 100.0)]);
            game.tick_creature_entities().unwrap();
            assert_eq!(log.lock().unwrap().post, [(225016, 100.0)]);
        });
    });
}

/// Damage and death hooks in the order the engine invokes them.
#[derive(Default)]
struct DeathOrder {
    heard: Mutex<Vec<(&'static str, i64)>>,
    cancel: std::sync::atomic::AtomicBool,
    revive: std::sync::atomic::AtomicBool,
    clear_cooldown: bool,
    refuse_main_protection: bool,
    dry_out_velocity: Mutex<Option<[f64; 3]>>,
}

struct DeathProbe(Arc<DeathOrder>);

#[mod_api::__subscriptions]
impl Hooks for DeathProbe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .heard
            .lock()
            .unwrap()
            .push(("incoming", i64::from(event.entity)));
        mod_api::Flow::Continue
    }
    fn living_damage_pre(&self, _: &mut dyn HitHost, event: &mut LivingDamagePreEvent<'_>) {
        if self.0.clear_cooldown {
            event
                .container
                .set_post_attack_invulnerability_ticks(0)
                .unwrap();
        }
        self.0
            .heard
            .lock()
            .unwrap()
            .push(("pre", i64::from(event.entity)));
    }
    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("post", entity));
    }
    fn living_use_totem(
        &self,
        _: &mut dyn HitHost,
        event: &mut mod_api::LivingUseTotemEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .heard
            .lock()
            .unwrap()
            .push(("totem", i64::from(event.entity)));
        if self.0.refuse_main_protection && event.hand == mod_api::Hand::Main {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
    fn living_death_post(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathPostEvent<'_>,
    ) {
        self.0
            .heard
            .lock()
            .unwrap()
            .push(("death-tail", i64::from(event.entity)));
    }
    fn living_death(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("death", entity));
        if event.kind == "minecraft:dolphin" && event.source.damage_type == "minecraft:dry_out" {
            *self.0.dry_out_velocity.lock().unwrap() = host
                .entity(event.entity)
                .and_then(|entity| entity.velocity(host));
        }
        if self.0.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            if self.0.revive.load(std::sync::atomic::Ordering::Relaxed) {
                let entity = host.entity(event.entity).unwrap();
                assert!(host.living(entity).unwrap().set_health(host, 5.0));
            }
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

/// A pig's 100-damage hit settles death inside the hurt. Two subsequent creature ticks verify
/// that the lifecycle sweep does not deliver another death or Post.
fn killing_blow(cancel: bool) -> (Vec<(&'static str, i64)>, f32) {
    let probe = Arc::new(DeathOrder::default());
    probe
        .cancel
        .store(cancel, std::sync::atomic::Ordering::Relaxed);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(probe.clone())),
    )]
    .into();
    let mut health = f32::NAN;
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let pig = mob(game, 225017, "minecraft:pig");
            game.mobs.push(pig);
            let victim = game.entity(225017).unwrap();
            assert_eq!(
                victim.hurt(game, "minecraft:magic", 100.0, None, None),
                Some(true)
            );
            game.tick_creature_entities().unwrap();
            game.tick_creature_entities().unwrap();
            health = game.mobs.iter().find(|m| m.id == 225017).unwrap().health;
        });
    });
    let heard = std::mem::take(&mut *probe.heard.lock().unwrap());
    (heard, health)
}

/// As at NeoForge's `hurtServer` tail: a killing blow's Post follows the death hooks.
#[test]
#[ignore = "daily"]
fn a_killing_blows_post_fires_once_after_the_death_hooks() {
    assert_eq!(
        killing_blow(false),
        (
            vec![
                ("incoming", 225017),
                ("pre", 225017),
                ("death", 225017),
                ("death-tail", 225017),
                ("post", 225017)
            ],
            0.0
        )
    );
}

/// A death a mod cancels: the pig remains cancelled at 0 health with `death_canceled`,
/// without a second Post or death on the next tick.
#[test]
#[ignore = "daily"]
fn a_cancelled_deaths_post_fires_once_after_the_death_hooks() {
    assert_eq!(
        killing_blow(true),
        (
            vec![
                ("incoming", 225017),
                ("pre", 225017),
                ("death", 225017),
                ("post", 225017)
            ],
            0.0
        )
    );
}

#[test]
#[ignore = "daily"]
fn guest_strike_posts_before_returning_from_the_authoritative_death() {
    use std::sync::atomic::Ordering::Relaxed;
    use sunrise_gameplay::game::native_player_death::tests::add_guest;
    let trace = Arc::new(DeathOrder::default());
    trace.cancel.store(true, Relaxed);
    trace.revive.store(true, Relaxed);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            add_guest(game);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.load_timeout = 0);
            let entity = i64::from(game.server.lan.as_ref().unwrap().hub.guests[0].entity_id);
            game.guest_struck_by(1, |game| {
                assert!(game.hurt_player_from(
                    100.0,
                    None,
                    "minecraft:generic",
                    sunrise_gameplay_common::game::creatures::KnownCause::None,
                    false
                ));
                assert_eq!(game.player.as_ref().unwrap().health.health(), 5.0);
                assert_eq!(
                    *trace.heard.lock().unwrap(),
                    [
                        ("incoming", entity),
                        ("pre", entity),
                        ("death", entity),
                        ("post", entity)
                    ]
                );
                trace.cancel.store(false, Relaxed);
                // A stronger hit in the same turn can kill the now-living body.
                assert!(game.hurt_player_from(
                    200.0,
                    None,
                    "minecraft:generic",
                    sunrise_gameplay_common::game::creatures::KnownCause::None,
                    false
                ));
                assert!(game.server.lan.as_ref().unwrap().hub.guests[0].dead);
                assert_eq!(trace.heard.lock().unwrap().last(), Some(&("post", entity)));
            })
            .unwrap();
            game.guest_die(0, 1);
            assert_eq!(
                *trace.heard.lock().unwrap(),
                [
                    ("incoming", entity),
                    ("pre", entity),
                    ("death", entity),
                    ("post", entity),
                    ("incoming", entity),
                    ("pre", entity),
                    ("death", entity),
                    ("death-tail", entity),
                    ("post", entity),
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_sweep_finishes_its_lethal_hit_before_the_next_hit() {
    let trace = Arc::new(DeathOrder::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 7;
            game.player = Some(player);
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:iron_sword", 1));
            let mut first = mob(game, 225031, "minecraft:pig");
            first.health = 0.5;
            let second = mob(game, 225032, "minecraft:pig");
            game.mobs.push(first);
            game.mobs.push(second);
            let area = sunrise_core::geometry::Aabb::new(
                Vec3::new(-1.0, -1.0, -1.0),
                Vec3::new(1.0, 2.0, 2.0),
            );
            game.sweep_damage(
                area,
                sunrise_gameplay::game::server_side::mace::Spared::Nothing,
                1.0,
            );
            assert_eq!(
                *trace.heard.lock().unwrap(),
                [
                    ("incoming", 225031),
                    ("pre", 225031),
                    ("death", 225031),
                    ("death-tail", 225031),
                    ("post", 225031),
                    ("incoming", 225032),
                    ("pre", 225032),
                    ("post", 225032),
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn projectile_killing_hit_posts_before_its_follow_up() {
    use super::native_entity_refs::tests::run_fresh_cases;
    use sunrise_sim::{
        mobs::Who,
        projectiles::{Hit, Projectile, ProjectileKind},
    };

    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::projectile_killing_hit_posts_before_its_follow_up",
        "SUNRISE_PROJECTILE_ORDER_CHILD",
        &["fireball"],
    ) {
        return;
    }
    let trace = Arc::new(DeathOrder::default());
    let mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.server.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            let center = Vec3::new(4.5, 80.0, 4.5);
            let (direct, blast) = (225034, 225035);
            let mut target = mob(game, direct, "minecraft:pig");
            target.position = center;
            target.health = 0.5;
            game.mobs.push(target);
            let mut bystander = mob(game, blast, "minecraft:pig");
            bystander.position = center.add_vector(Vec3::new(1.5, 0.0, 0.0));
            game.mobs.push(bystander);
            let mut shot =
                Projectile::new(ProjectileKind::LargeFireball { power: 1 }, center, None);
            shot.id = 225036;
            game.projectile_hit(
                &shot,
                Hit::Entity {
                    who: Who::Mob(direct),
                    at: center,
                },
            )
            .unwrap();
            assert_eq!(
                *trace.heard.lock().unwrap(),
                [
                    ("incoming", i64::from(direct)),
                    ("pre", i64::from(direct)),
                    ("death", i64::from(direct)),
                    ("death-tail", i64::from(direct)),
                    ("post", i64::from(direct)),
                    ("incoming", i64::from(blast)),
                    ("pre", i64::from(blast)),
                    ("post", i64::from(blast)),
                ],
                "the direct hit completes before the blast starts another hurt",
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn mounted_fall_posts_before_the_next_rider() {
    use super::{
        lan_vehicles::controller_tests::guest_horse_at_start_of_fall,
        native_entity_refs::tests::run_fresh_cases,
    };
    use std::sync::atomic::Ordering::Relaxed;
    use sunrise_protocol::packets::play::MoveVehicleRequest;

    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::mounted_fall_posts_before_the_next_rider",
        "SUNRISE_MOUNTED_FALL_ORDER_CHILD",
        &["horse"],
    ) {
        return;
    }
    let trace = Arc::new(DeathOrder::default());
    trace.cancel.store(true, Relaxed);
    trace.revive.store(true, Relaxed);
    let mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (horse, _packets) = guest_horse_at_start_of_fall(game);
            game.mobs[0].health = 0.5;
            let rider = game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.health = 0.5;
                g.entity_id
            });
            for (y, on_ground) in [
                (209.0, false),
                (207.0, false),
                (204.0, false),
                (200.0, true),
            ] {
                game.guest_move_vehicle(
                    2,
                    &MoveVehicleRequest {
                        position: [4.5, y, 4.5],
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground,
                    },
                );
            }
            let horse = i64::from(horse);
            let rider = i64::from(rider);
            assert_eq!(
                *trace.heard.lock().unwrap(),
                [
                    ("incoming", horse),
                    ("pre", horse),
                    ("death", horse),
                    ("post", horse),
                    ("incoming", rider),
                    ("pre", rider),
                    ("death", rider),
                    ("post", rider)
                ],
                "the horse settles its own hurt before calling causeFallDamage on its rider",
            );
            assert_eq!(game.server.lan.as_ref().unwrap().hub.guests[0].health, 5.0);
            assert_eq!(game.mobs[0].health, 5.0);
            assert_eq!(game.mobs[0].fall_distance, 0.0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn splash_water_finishes_a_lethal_hit_before_the_next_target() {
    let trace = Arc::new(DeathOrder::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let mut first = mob(game, 225041, "minecraft:blaze");
            first.health = 0.5;
            let second = mob(game, 225042, "minecraft:blaze");
            game.mobs.push(first);
            game.mobs.push(second);
            let water = ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:water");
            game.affect_entities_around(&water, Vec3::ZERO, false);
            assert_eq!(
                *trace.heard.lock().unwrap(),
                [
                    ("incoming", 225041),
                    ("pre", 225041),
                    ("death", 225041),
                    ("death-tail", 225041),
                    ("post", 225041),
                    ("incoming", 225042),
                    ("pre", 225042),
                    ("post", 225042),
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn squid_finishes_the_lethal_hit_its_subclass_reports_false() {
    let trace = Arc::new(DeathOrder::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let mut squid = mob(game, 225045, "minecraft:squid");
            squid.health = 0.5;
            game.mobs.push(squid);
            assert!(!game.mob_hits_mob(225045, 1.0, Damage::of(DamageKind::Magic)));
            assert_eq!(
                *trace.heard.lock().unwrap(),
                [
                    ("incoming", 225045),
                    ("pre", 225045),
                    ("death", 225045),
                    ("death-tail", 225045),
                    ("post", 225045),
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_death_listener_without_post_runs_before_the_next_hurt() {
    struct DeathOnly(Arc<Mutex<Vec<(&'static str, i64)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for DeathOnly {
        fn living_incoming_damage(
            &self,
            _: &mut dyn HitHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> mod_api::Flow {
            self.0
                .lock()
                .unwrap()
                .push(("incoming", i64::from(event.entity)));
            mod_api::Flow::Continue
        }

        fn living_death(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LivingDeathEvent<'_>,
        ) -> mod_api::Flow {
            self.0
                .lock()
                .unwrap()
                .push(("death", i64::from(event.entity)));
            mod_api::Flow::Continue
        }
    }

    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "death-without-post",
        Box::new(DeathOnly(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let mut squid = mob(game, 225046, "minecraft:squid");
            squid.health = 0.5;
            let pig = mob(game, 225047, "minecraft:pig");
            game.mobs.push(squid);
            game.mobs.push(pig);
            let source = Damage::of(DamageKind::Magic);
            // The squid can return false after its superclass accepted the hit.
            assert!(!game.mob_hits_mob(225046, 1.0, source));
            assert!(game.mob_hits_mob(225047, 1.0, source));
            assert_eq!(
                *trace.lock().unwrap(),
                [
                    ("incoming", 225046),
                    ("death", 225046),
                    ("incoming", 225047)
                ],
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_falling_block_finishes_each_hit_before_the_next_victim() {
    let trace = Arc::new(DeathOrder::default());
    let mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            use sunrise_core::position::BlockPos;
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let at = Vec3::new(0.5, 64.0, 0.5);
            let mut first = mob(game, 225048, "minecraft:pig");
            first.health = 0.5;
            first.position = at;
            let mut second = mob(game, 225049, "minecraft:pig");
            second.position = at;
            game.mobs.push(first);
            game.mobs.push(second);
            let mut anvil = sunrise_gameplay_common::game::FallingBlock {
                size_answer: None,
                attachment_lifetime: Default::default(),
                rotation: [0.0; 2],
                join: Default::default(),
                id: 225050,
                blocks_building: true,
                state: game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:anvil")
                    .unwrap(),
                start_pos: BlockPos::containing(at),
                position: at,
                previous: at,
                velocity: Vec3::ZERO,
                age: 1,
                no_gravity: false,
                portal: Default::default(),
                drop_item: true,
                fall_distance: 2.0,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                fire_ticks: 0,
                sync_velocity: false,
                stuck: Vec3::ZERO,
                damage: sunrise_sim::falling_block::Damage::anvil(),
                saved: None,
            };
            game.falling_block_impact(&mut anvil);
            assert_eq!(
                *trace.heard.lock().unwrap(),
                [
                    ("incoming", 225048),
                    ("pre", 225048),
                    ("death", 225048),
                    ("death-tail", 225048),
                    ("post", 225048),
                    ("incoming", 225049),
                    ("pre", 225049),
                    ("post", 225049),
                ],
            );
        });
    });
}

#[derive(Debug, PartialEq, Eq)]
enum PufferfishMoment {
    Incoming(i32),
    Death(Vec<(i32, i32)>),
    Post(i32),
    FishTickPost(Vec<(i32, i32)>),
}

struct PufferfishProbe {
    fish: u32,
    first: u32,
    trace: Arc<Mutex<Vec<PufferfishMoment>>>,
}

fn active_poison(host: &dyn mod_api::ServerHost, victim: mod_api::LivingRef) -> Vec<(i32, i32)> {
    let poison = host.effect_key("minecraft:poison").unwrap();
    let mut effects = Vec::new();
    assert_eq!(
        victim.visit_effects(host, &mut |effect| {
            if effect.id == poison {
                effects.push((effect.duration, effect.amplifier));
            }
            core::ops::ControlFlow::Continue(())
        }),
        Ok(core::ops::ControlFlow::Continue(()))
    );
    effects
}

#[mod_api::__subscriptions]
impl Hooks for PufferfishProbe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        if event.source.causing == self.fish as i32
            && event.source.damage_type == "minecraft:mob_attack"
        {
            self.trace
                .lock()
                .unwrap()
                .push(PufferfishMoment::Incoming(event.entity));
        }
        mod_api::Flow::Continue
    }

    fn living_death(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        if event.entity != self.first || event.source.causing != self.fish as i32 {
            return mod_api::Flow::Continue;
        }
        let victim = host.living(host.entity(self.first).unwrap()).unwrap();
        self.trace
            .lock()
            .unwrap()
            .push(PufferfishMoment::Death(active_poison(host, victim)));
        // Keep the victim available for the poison and the fish's own Post read.
        assert!(victim.set_health(host, 5.0));
        mod_api::Flow::Cancel
    }

    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        if event.source.causing == self.fish as i32
            && event.source.damage_type == "minecraft:mob_attack"
        {
            self.trace
                .lock()
                .unwrap()
                .push(PufferfishMoment::Post(event.entity));
        }
    }

    fn entity_tick_post(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        if event.entity == self.fish
            && let mod_api::HostSide::Server(host) = host
        {
            let victim = host.living(host.entity(self.first).unwrap()).unwrap();
            self.trace
                .lock()
                .unwrap()
                .push(PufferfishMoment::FishTickPost(active_poison(host, victim)));
        }
    }
}

#[test]
#[ignore = "daily"]
fn borrowed_pufferfish_posts_before_poison_and_the_next_sting() {
    use super::native_entity_refs::tests::run_fresh_cases;
    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::borrowed_pufferfish_posts_before_poison_and_the_next_sting",
        "SUNRISE_BORROWED_PUFFERFISH_CHILD",
        &["sting"],
    ) {
        return;
    }
    let (fish, first, second) = (225051, 225052, 225053);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "pufferfish-order",
        Box::new(PufferfishProbe {
            fish,
            first,
            trace: Arc::clone(&trace),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.server.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            // Same chunk/section, in the order the contact query visits them.
            let mut puffer = mob(game, fish, "minecraft:pufferfish");
            puffer.position = Vec3::new(1.5, 80.0, 1.5);
            puffer.extra.size = 2;
            puffer.no_ai = true;
            puffer.no_gravity = true;
            let age = puffer.age;
            game.mobs.push(puffer);
            for (id, x) in [(first, 1.25), (second, 1.75)] {
                let mut pig = mob(game, id, "minecraft:pig");
                pig.position = Vec3::new(x, 80.0, 1.5);
                pig.no_ai = true;
                pig.no_gravity = true;
                if id == first {
                    pig.health = 0.5;
                }
                game.mobs.push(pig);
            }

            game.tick_creature_entities().unwrap();

            use PufferfishMoment::*;
            assert_eq!(
                *trace.lock().unwrap(),
                [
                    Incoming(first as i32),
                    Death(vec![]),
                    Post(first as i32),
                    Incoming(second as i32),
                    Post(second as i32),
                    FishTickPost(vec![(120, 0)])
                ]
            );
            let revived = game.mobs.iter().find(|m| m.id == first).unwrap();
            assert!(revived.alive(), "the death veto revived the first victim");
            let puffer = game.mobs.iter().find(|m| m.id == fish).unwrap();
            assert_eq!(
                puffer.age,
                age + 1,
                "resuming a sting must not rerun the fish tick"
            );
        });
    });
}

#[derive(Debug, PartialEq)]
enum BeeAiMoment {
    DrowningIncoming,
    DrowningPost,
    StingerIncoming(f32),
    StingerPost,
    AngerCleared,
}

struct BeeAiProbe {
    id: u32,
    mark_stung_on_drowning: bool,
    trace: Arc<Mutex<Vec<BeeAiMoment>>>,
}

#[mod_api::__subscriptions]
impl Hooks for BeeAiProbe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        if event.entity as u32 == self.id {
            let moment = match event.source.damage_type {
                "minecraft:drown" => {
                    event
                        .container
                        .set_post_attack_invulnerability_ticks(0)
                        .unwrap();
                    BeeAiMoment::DrowningIncoming
                }
                "minecraft:generic" => BeeAiMoment::StingerIncoming(event.container.original()),
                _ => return mod_api::Flow::Continue,
            };
            self.trace.lock().unwrap().push(moment);
        }
        mod_api::Flow::Continue
    }

    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        if event.entity as u32 == self.id {
            let moment = match event.source.damage_type {
                "minecraft:drown" => BeeAiMoment::DrowningPost,
                "minecraft:generic" => BeeAiMoment::StingerPost,
                _ => return,
            };
            self.trace.lock().unwrap().push(moment);
        }
    }

    fn living_death(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        if event.entity != self.id {
            return mod_api::Flow::Continue;
        }
        let entity = host.entity(self.id).unwrap();
        // Expiry must hurt for this restored health, not health from before drowning.
        assert!(host.living(entity).unwrap().set_health(host, 4.0));
        if self.mark_stung_on_drowning && event.source.damage_type == "minecraft:drown" {
            let mut patch = mod_api::Compound::default();
            patch.insert("HasStung".into(), mod_api::Tag::Byte(1));
            assert!(entity.merge_nbt(host, patch));
        }
        mod_api::Flow::Cancel
    }

    fn living_change_target(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::LivingChangeTargetEvent<'_>,
    ) -> mod_api::Flow {
        if event.mob.entity().id() == self.id && event.new_target.is_none() {
            self.trace.lock().unwrap().push(BeeAiMoment::AngerCleared);
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn borrowed_bee_posts_at_both_server_ai_hurts() {
    use super::native_entity_refs::tests::run_fresh_cases;
    use sunrise_sim::mobs::{Who, bee};

    const CASE: &str = "SUNRISE_BORROWED_BEE_DAMAGE_CHILD";
    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::borrowed_bee_posts_at_both_server_ai_hurts",
        CASE,
        &["expiry", "snapshot"],
    ) {
        return;
    }
    let snapshot = std::env::var(CASE).unwrap() == "snapshot";
    let id = 225054;
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "bee-ai-hurt-order",
        Box::new(BeeAiProbe {
            id,
            mark_stung_on_drowning: snapshot,
            trace: Arc::clone(&trace),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.server.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            let water = game
                .pack
                .assets
                .registry
                .default_state("minecraft:water")
                .unwrap();
            game.world.set(1, 80, 1, water).unwrap();
            let mut body = mob(game, id, "minecraft:bee");
            body.position = Vec3::new(1.5, 80.0, 1.5);
            body.health = 0.5;
            body.ai.goals.clear();
            body.ai.targets.clear();
            bee::set_has_stung(&mut body, !snapshot);
            let bee = body.extra.bee.as_mut().unwrap();
            bee.under_water = 20;
            if !snapshot {
                // At 1200 the expiry roll is certain. Anger at a departed target
                // is cleared by updatePersistentAnger, the tail of the custom step.
                bee.time_since_sting = 1199;
                let departed = Some(Who::Mob(id + 1));
                bee.angry_at = departed;
                bee.anger_end = game.world.game_time + 100;
                body.set_target(departed);
            }
            game.mobs.push(body);
            game.entity_section_joined(id, [1.5, 80.0, 1.5]);

            game.tick_creature_entities().unwrap();

            let body = game.mobs.iter().find(|m| m.id == id).unwrap();
            let bee = bee::get(body).unwrap();
            use BeeAiMoment::*;
            if snapshot {
                assert!(
                    bee::has_stung(body),
                    "the death listener changed the live flag"
                );
                assert_eq!(
                    bee.time_since_sting, 0,
                    "the step reads the pre-drowning flag"
                );
                assert_eq!(*trace.lock().unwrap(), [DrowningIncoming, DrowningPost]);
            } else {
                assert_eq!(
                    *trace.lock().unwrap(),
                    [
                        DrowningIncoming,
                        DrowningPost,
                        StingerIncoming(4.0),
                        StingerPost,
                        AngerCleared
                    ]
                );
                assert_eq!(bee.time_since_sting, 1200);
                assert_eq!(bee.under_water, 21);
            }
            assert_eq!(
                bee.ticks_without_nectar, 1,
                "the custom step's tail runs once"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn borrowed_base_tick_damage_settles_before_the_next_mob_tick() {
    for hazard in ["burn", "air", "effect", "contact", "dolphin dry-out"] {
        let trace = Arc::new(DeathOrder::default());
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "death-order",
            Box::new(DeathProbe(trace.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.build_registries();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                if hazard == "contact" {
                    let cactus = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:cactus")
                        .unwrap();
                    game.world.set(1, 80, 1, cactus).unwrap();
                }
                let mut first = mob(
                    game,
                    225043,
                    match hazard {
                        "air" => "minecraft:axolotl",
                        "dolphin dry-out" => "minecraft:dolphin",
                        _ => "minecraft:pig",
                    },
                );
                let mut second = mob(game, 225044, "minecraft:axolotl");
                first.position = Vec3::new(1.0, 80.0, 1.0);
                second.position = Vec3::new(3.0, 80.0, 1.0);
                first.health = 0.5;
                match hazard {
                    "burn" => first.fire_ticks = 20,
                    "effect" => first.add_effect("wither", 40, 0),
                    "contact" => first.no_ai = true,
                    "dolphin dry-out" => {
                        let stone = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:stone")
                            .unwrap();
                        game.world.set(1, 79, 1, stone).unwrap();
                        first.position = Vec3::new(1.5, 80.0, 1.5);
                        // Travel lands on stone before dry-out, with no gravity
                        // or AI speed to change the settled velocity again.
                        first.velocity = Vec3::new(0.0, -0.1, 0.0);
                        first.no_gravity = true;
                        first.attrs.speed = 0.0;
                        first.extra.counter = 1;
                    }
                    _ => first.air = -19,
                }
                second.air = -19;
                game.mobs.push(first);
                game.mobs.push(second);
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    *trace.heard.lock().unwrap(),
                    [
                        ("incoming", 225043),
                        ("pre", 225043),
                        ("death", 225043),
                        ("death-tail", 225043),
                        ("post", 225043),
                        ("incoming", 225044),
                        ("pre", 225044),
                        ("post", 225044),
                    ],
                    "{hazard}"
                );
                if hazard == "dolphin dry-out" {
                    assert_eq!(*trace.dry_out_velocity.lock().unwrap(), Some([0.0; 3]));
                    let dolphin = game.mobs.iter().find(|m| m.id == 225043).unwrap();
                    assert_eq!(dolphin.velocity.y, 0.5, "one hop after death and Post");
                    assert!(!dolphin.ground.on_ground);
                    assert_eq!(dolphin.extra.counter, 0, "dry-out ran only once");
                }
            });
        });
    }
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn borrowed_mod_effect_posts_before_the_next_action() {
    use super::native_entity_refs::tests::run_fresh_cases;
    use std::sync::atomic::Ordering::Relaxed;
    use sunrise_gameplay_common::game::client_side::potions::tests::effect_test_mod;

    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::borrowed_mod_effect_posts_before_the_next_action",
        "SUNRISE_BORROWED_EFFECT_DAMAGE_CHILD",
        &["effect"],
    ) {
        return;
    }
    let trace = Arc::new(DeathOrder {
        clear_cooldown: true,
        ..DeathOrder::default()
    });
    trace.cancel.store(true, Relaxed);
    trace.revive.store(true, Relaxed);
    let applied = effect_test_mod([mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]);
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.build_registries();
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let mut pig = mob(game, 225046, "minecraft:pig");
        pig.position = Vec3::new(1.0, 80.0, 1.0);
        pig.health = 0.5;
        let effect = sunrise_level::brewing::effect("test:effect").unwrap().id;
        pig.add_effect(effect, 10, 4);
        game.mobs.push(pig);

        game.tick_creature_entities().unwrap();

        assert_eq!(
            *trace.heard.lock().unwrap(),
            [
                ("incoming", 225046),
                ("pre", 225046),
                ("death", 225046),
                ("post", 225046),
                ("incoming", 225046),
                ("pre", 225046),
                ("post", 225046),
            ]
        );
        assert_eq!(applied.load(Relaxed), 1);
        let pig = game.mobs.iter().find(|m| m.id == 225046).unwrap();
        assert_eq!(pig.health, 4.0, "revived to 5, lost 2, healed once for 1");
        assert_eq!(
            sunrise_gameplay_common::game::creatures::source_damage_type(
                pig.last_damage.unwrap().0
            ),
            "minecraft:indirect_magic"
        );
        assert_eq!(pig.extra.effects[0].duration, 9);
    });
}

#[test]
#[ignore = "daily"]
fn guest_protection_consumes_the_item_and_posts_inside_the_hit() {
    use sunrise_gameplay::game::native_player_death::tests::add_guest;
    let trace = Arc::new(DeathOrder {
        refuse_main_protection: true,
        ..Default::default()
    });
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            add_guest(game);
            let main = ItemStack::new("minecraft:totem_of_undying", 1);
            let mut off = ItemStack::new("minecraft:stick", 2);
            off.data_mut().set_other(
                "minecraft:death_protection",
                Some(sunrise_core::nbt::Tag::compound(
                    [] as [(&str, sunrise_core::nbt::Tag); 0]
                )),
            );
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.load_timeout = 0;
                    guest.inventory[guest.selected] = Some(main.clone());
                    guest.offhand = Some(off);
                });
            let entity = i64::from(game.server.lan.as_ref().unwrap().hub.guests[0].entity_id);
            game.guest_struck_by(1, |game| {
                assert!(game.hurt_player_from(
                    100.0,
                    None,
                    "minecraft:generic",
                    sunrise_gameplay_common::game::creatures::KnownCause::None,
                    false
                ));
                assert_eq!(game.inventory[game.selected].as_ref(), Some(&main));
                assert_eq!(game.offhand.as_ref().unwrap().count, 1);
                assert_eq!(game.player.as_ref().unwrap().health.health(), 1.0);
                assert_eq!(
                    *trace.heard.lock().unwrap(),
                    [
                        ("incoming", entity),
                        ("pre", entity),
                        ("totem", entity),
                        ("totem", entity),
                        ("post", entity)
                    ]
                );
            })
            .unwrap();
            assert!(!game.server.lan.as_ref().unwrap().hub.guests[0].dead);
        });
    });
}

#[test]
#[ignore = "daily"]
fn guest_burning_hit_posts_before_the_void_hurt_in_the_same_tick() {
    use sunrise_gameplay::game::native_player_death::tests::add_guest;
    let trace = Arc::new(DeathOrder::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            add_guest(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let at = Vec3::new(1.0, game.world.below_world_y() - 1.0, 1.0);
            let mut body = PlayerMotion::new(game.motion, at).unwrap();
            body.fire_ticks = 20;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                    guest.position = at.components();
                    guest.health = 0.5;
                    guest.motion = Some(Box::new(body));
                    guest.inventory[guest.selected] =
                        Some(ItemStack::new("minecraft:totem_of_undying", 1));
                });
            let entity = i64::from(game.server.lan.as_ref().unwrap().hub.guests[0].entity_id);
            game.guest_server_tick(1, true);
            assert!(trace.heard.lock().unwrap().starts_with(&[
                ("incoming", entity),
                ("pre", entity),
                ("totem", entity),
                ("post", entity),
                ("incoming", entity),
            ]));
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert!(guest.inventory[guest.selected].is_none());
        });
    });
}

#[derive(Debug, PartialEq)]
enum MobLandingMoment {
    Trample,
    Death(Option<mod_api::Tag>, Option<[f64; 3]>),
    Post,
    Repath,
    NextTargetGoal,
    NextGoal,
    NextHit,
    TickFinished(Option<mod_api::Tag>),
}

type MobLandingTrace = Arc<Mutex<Vec<MobLandingMoment>>>;

struct MobLandingProbe {
    id: u32,
    next_hit: Option<u32>,
    trace: MobLandingTrace,
}

#[mod_api::__subscriptions]
impl Hooks for MobLandingProbe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        if Some(event.entity as u32) == self.next_hit {
            self.trace.lock().unwrap().push(MobLandingMoment::NextHit);
        }
        mod_api::Flow::Continue
    }

    fn farmland_trample(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::FarmlandTrampleEvent,
    ) -> mod_api::Flow {
        if event.entity.entity().id() == self.id {
            self.trace.lock().unwrap().push(MobLandingMoment::Trample);
        }
        mod_api::Flow::Continue
    }

    fn living_death(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        if event.entity == self.id && event.source.damage_type == "minecraft:fall" {
            let entity = host.entity(self.id).unwrap();
            let distance = entity
                .nbt(host)
                .unwrap()
                .get(&"fall_distance".into())
                .cloned();
            let velocity = entity.velocity(host);
            self.trace
                .lock()
                .unwrap()
                .push(MobLandingMoment::Death(distance, velocity));
            // Keep the landed mob alive so its remaining goals and travel can be observed.
            assert!(host.living(entity).unwrap().set_health(host, 5.0));
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }

    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        if event.entity as u32 == self.id && event.source.damage_type == "minecraft:fall" {
            self.trace.lock().unwrap().push(MobLandingMoment::Post);
        }
    }

    fn entity_tick_post(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        if event.entity == self.id
            && let mod_api::HostSide::Server(host) = host
        {
            let entity = host.entity(self.id).unwrap();
            let distance = entity
                .nbt(host)
                .unwrap()
                .get(&"fall_distance".into())
                .cloned();
            self.trace
                .lock()
                .unwrap()
                .push(MobLandingMoment::TickFinished(distance));
        }
    }
}

#[test]
#[ignore = "daily"]
fn borrowed_mob_landing_posts_before_the_travel_tail() {
    use super::native_entity_refs::tests::run_fresh_cases;
    const CASE: &str = "SUNRISE_MOB_LANDING_DAMAGE_CHILD";
    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::borrowed_mob_landing_posts_before_the_travel_tail",
        CASE,
        &["stone", "farmland"],
    ) {
        return;
    }
    let farmland = std::env::var(CASE).unwrap() == "farmland";
    let id = 225047;
    let next_hit = 225050;
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "mob-landing-order",
        Box::new(MobLandingProbe {
            id,
            next_hit: Some(next_hit),
            trace: Arc::clone(&trace),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.server.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            let registry = game.pack.assets.registry;
            let block = registry
                .default_state(if farmland {
                    "minecraft:farmland"
                } else {
                    "minecraft:stone"
                })
                .unwrap();
            game.world.set(1, 63, 1, block).unwrap();
            let mut pig = mob(game, id, "minecraft:pig");
            pig.position = Vec3::new(1.5, 64.125, 1.5);
            pig.velocity.y = -0.5;
            pig.fall_distance = 6.0;
            pig.health = 0.5;
            pig.ai.goals = Default::default();
            pig.ai.targets = Default::default();
            game.mobs.push(pig);
            game.entity_section_joined(id, [1.5, 64.125, 1.5]);
            let mut drying = mob(game, next_hit, "minecraft:axolotl");
            drying.position = Vec3::new(3.5, 64.125, 1.5);
            drying.air = -19;
            game.mobs.push(drying);

            game.tick_creature_entities().unwrap();

            let mut expected = Vec::new();
            let fall: f64 = if farmland {
                expected.push(MobLandingMoment::Trample);
                let dirt = registry.default_state("minecraft:dirt").unwrap();
                assert_eq!(game.world.block(1, 63, 1), dirt);
                6.1875
            } else {
                6.125
            };
            expected.extend([
                MobLandingMoment::Death(
                    Some(mod_api::Tag::Double(fall.to_bits())),
                    Some([0.0, -0.5, 0.0]),
                ),
                MobLandingMoment::Post,
                // The pig's EntityTickEvent.Post closes its own turn, before the next mob's.
                MobLandingMoment::TickFinished(Some(mod_api::Tag::Double(0))),
                MobLandingMoment::NextHit,
            ]);
            assert_eq!(*trace.lock().unwrap(), expected);
            let pig = &game.mobs[0];
            assert_eq!(pig.position.y, 64.0);
            assert_eq!(
                pig.velocity.y,
                -0.08 * f64::from(0.98_f32),
                "one travel tail after the hurt"
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_dolphin_goal_move_posts_before_the_next_goal() {
    use super::native_entity_refs::tests::run_fresh_cases;
    use sunrise_sim::mobs::{
        dolphin::FollowPlayerRidden,
        goals::{BreathAir, Cx, Goal},
        living::TravelPause,
    };

    const CASE: &str = "SUNRISE_DOLPHIN_GOAL_DAMAGE_CHILD";
    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::a_dolphin_goal_move_posts_before_the_next_goal",
        CASE,
        &["follow", "breath", "pig-target-breath"],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let breath = case != "follow";
    let target = case == "pig-target-breath";

    // Use the real goal, observing its completion separately from the next goal's tick.
    #[derive(Clone, Debug)]
    struct Follow(FollowPlayerRidden, MobLandingTrace);
    impl Goal for Follow {
        fn flags(&self) -> u8 {
            self.0.flags()
        }
        fn can_use(&mut self, mob: &mut Mob, cx: &mut Cx) -> bool {
            self.0.can_use(mob, cx)
        }
        fn start(&mut self, mob: &mut Mob, cx: &mut Cx) {
            self.0.start(mob, cx);
        }
        fn tick_until_movement_pause(&mut self, mob: &mut Mob, cx: &mut Cx) -> Option<TravelPause> {
            self.0.tick_until_movement_pause(mob, cx)
        }
        fn after_movement(&mut self, mob: &mut Mob, cx: &mut Cx) -> bool {
            if !self.0.after_movement(mob, cx) {
                return false;
            }
            self.1.lock().unwrap().push(MobLandingMoment::Repath);
            true
        }
        fn box_clone(&self) -> Box<dyn Goal> {
            Box::new(self.clone())
        }
    }

    #[derive(Clone, Debug)]
    struct FollowingGoal {
        trace: MobLandingTrace,
        target: bool,
    }
    impl Goal for FollowingGoal {
        fn flags(&self) -> u8 {
            0
        }
        fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
            true
        }
        fn tick(&mut self, _: &mut Mob, _: &mut Cx) {
            self.trace.lock().unwrap().push(if self.target {
                MobLandingMoment::NextTargetGoal
            } else {
                MobLandingMoment::NextGoal
            });
        }
        fn box_clone(&self) -> Box<dyn Goal> {
            Box::new(self.clone())
        }
    }

    struct KeepLowAir(u32);

    #[mod_api::__subscriptions]
    impl Hooks for KeepLowAir {
        fn living_breathe(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LivingBreatheEvent,
        ) {
            if event.entity.entity().id() == self.0 {
                // Keep the low air on this land tick before BreathAir checks it.
                event.can_breathe = true;
                event.refill = 0;
            }
        }
    }

    let id = 225048;
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut mods = vec![mod_api::ModEntry::new(
        "goal-landing-order",
        Box::new(MobLandingProbe {
            id,
            next_hit: None,
            trace: Arc::clone(&trace),
        }),
    )];
    if breath {
        // Each fresh child needs the bridge that lets baseTick hear KeepLowAir.
        crate::entry::install_client_hooks();
        mods.push(mod_api::ModEntry::new(
            "keep-low-air",
            Box::new(KeepLowAir(id)),
        ));
    }
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.server.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 79, 1, stone).unwrap();

            if !breath {
                let at = Vec3::new(3.5, 80.0, 1.5);
                let mut boat = sunrise_gameplay_common::game::boats_common::Boat::new(
                    225049,
                    "oak".into(),
                    at,
                    0.0,
                );
                boat.velocity.x = 0.2;
                game.riding = Some(boat.id);
                game.boats.push(boat);
                game.player = Some(PlayerMotion::new(game.motion, at).unwrap());
            }

            let kind = if target {
                "minecraft:pig"
            } else {
                "minecraft:dolphin"
            };
            let mut mover = mob(game, id, kind);
            mover.position = Vec3::new(1.5, 80.0, 1.5);
            mover.velocity.y = -0.5;
            mover.fall_distance = 6.0;
            mover.health = 0.5;
            mover.ai.goals.clear();
            mover.ai.targets.clear();
            if breath {
                mover.air = 20;
                if !target {
                    mover.ai.goals.add(0, BreathAir::default());
                }
            } else {
                mover.ai.goals.add(
                    0,
                    Follow(FollowPlayerRidden::new(false), Arc::clone(&trace)),
                );
            }
            mover.ai.goals.add(
                1,
                FollowingGoal {
                    trace: Arc::clone(&trace),
                    target: false,
                },
            );
            game.mobs.push(mover);
            game.entity_section_joined(id, [1.5, 80.0, 1.5]);
            if target {
                let handle = game.mob(game.entity(id).unwrap()).unwrap();
                assert!(handle.add_target_goal(game, 0, &mod_api::GoalSpec::BreathAir));
                game.mobs[0].ai.targets.add(
                    1,
                    FollowingGoal {
                        trace: Arc::clone(&trace),
                        target: true,
                    },
                );
            }

            game.tick_creature_entities().unwrap();

            let mut expected = vec![
                MobLandingMoment::Death(
                    Some(mod_api::Tag::Double(6.0_f64.to_bits())),
                    Some([0.0, -0.5, 0.0]),
                ),
                MobLandingMoment::Post,
            ];
            if !breath {
                expected.push(MobLandingMoment::Repath);
            }
            if target {
                expected.push(MobLandingMoment::NextTargetGoal);
            }
            expected.extend([
                MobLandingMoment::NextGoal,
                MobLandingMoment::TickFinished(Some(mod_api::Tag::Double(0))),
            ]);
            assert_eq!(*trace.lock().unwrap(), expected);
        })
    });
}

#[derive(Debug, PartialEq)]
enum GuestLandingMoment {
    Death(u32, Option<[f64; 3]>, Option<mod_api::Tag>),
    Post(i32),
    NextMove(u32, Option<mod_api::Tag>),
}

struct GuestLandingProbe(Arc<Mutex<Vec<GuestLandingMoment>>>);

#[mod_api::__subscriptions]
impl Hooks for GuestLandingProbe {
    fn living_death(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        if event.source.damage_type != "minecraft:fall" {
            return mod_api::Flow::Continue;
        }
        let player = host.entity(event.entity).unwrap();
        let position = player.position(host);
        // This projection reads the acting body, not the guest's stored record.
        let distance = player
            .nbt(host)
            .unwrap()
            .get(&"fall_distance".into())
            .cloned();
        self.0
            .lock()
            .unwrap()
            .push(GuestLandingMoment::Death(event.entity, position, distance));
        // Let the following accepted move run on the same body.
        assert!(host.living(player).unwrap().set_health(host, 5.0));
        mod_api::Flow::Cancel
    }

    fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push(GuestLandingMoment::Post(event.entity));
    }

    fn entity_entering_section(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityEnteringSectionEvent,
    ) {
        if let mod_api::HostSide::Server(host) = host
            && event.chunk_changed
        {
            let distance = event
                .entity
                .nbt(host)
                .unwrap()
                .get(&"fall_distance".into())
                .cloned();
            self.0
                .lock()
                .unwrap()
                .push(GuestLandingMoment::NextMove(event.entity.id(), distance));
        }
    }
}

#[test]
#[ignore = "daily"]
fn guest_landing_posts_before_its_ground_hit() {
    use sunrise_gameplay::game::native_player_death::tests::add_guest;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::lan::{guest::GuestMove, hub::CommittedMove};

    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "landing-order",
        Box::new(GuestLandingProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            add_guest(game);
            for x in [0, 1] {
                game.server
                    .streaming
                    .insert_remote(&mut game.common.world, Chunk::empty(x, 0));
            }
            let honey = game
                .pack
                .assets
                .registry
                .default_state("minecraft:honey_block")
                .unwrap();
            game.world.set(15, 78, 1, honey).unwrap();
            let landing = Vec3::new(15.5, 78.9375, 1.5);
            let start = Vec3::new(landing.x, landing.y + 0.25, landing.z);
            let next = [16.5, landing.y, landing.z];
            let mut body = PlayerMotion::new(game.motion, start).unwrap();
            body.fall_distance = 10.0;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                    guest.place(start.components());
                    // Even honey's reduced hit kills this body.
                    guest.health = 0.5;
                    guest.motion = Some(Box::new(body));
                });
            let entity = game.server.lan.as_ref().unwrap().hub.guests[0].entity_id;
            game.entity_section_joined(entity as u32, start.components());
            // Fall handling belongs to each accepted packet, not to the body tick's move history.
            for (from, position, on_ground) in [
                (start.components(), landing.components(), true),
                (landing.components(), next, false),
            ] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.accept_move(position);
                        guest.on_ground = on_ground;
                    });
                game.guest_accepted_move(
                    1,
                    CommittedMove {
                        from,
                        horizontal_collision: false,
                        jump_yaw: None,
                        movement: GuestMove {
                            position,
                            delta: std::array::from_fn(|axis| position[axis] - from[axis]),
                            on_ground,
                            sneaking: false,
                        },
                    },
                );
            }
            assert_eq!(
                *trace.lock().unwrap(),
                [
                    GuestLandingMoment::Death(
                        entity as u32,
                        Some(landing.components()),
                        Some(mod_api::Tag::Double(10.25_f64.to_bits())),
                    ),
                    GuestLandingMoment::Post(entity),
                    GuestLandingMoment::NextMove(entity as u32, Some(mod_api::Tag::Double(0))),
                ],
                "death sees the landing before its fall reset; the next move sees the finished tail",
            );
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.position, next);
            assert_eq!(guest.motion.as_ref().unwrap().fall_distance, 0.0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_guest_move_settles_fall_before_resuming() {
    use super::native_entity_refs::tests::run_fresh_cases;
    use sunrise_gameplay::game::native_player_death::tests::add_guest;
    use sunrise_level::chunk::Chunk;
    use sunrise_world::entity_move::MoverType;

    if run_fresh_cases(
        "cut_layout::game::server_side::native_damage_tests::ordinary_guest_move_settles_fall_before_resuming",
        "SUNRISE_ORDINARY_GUEST_LANDING_CHILD",
        &["honey"],
    ) {
        return;
    }
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "landing-order",
        Box::new(GuestLandingProbe(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            add_guest(game);
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            let honey = game
                .pack
                .assets
                .registry
                .default_state("minecraft:honey_block")
                .unwrap();
            game.world.set(1, 78, 1, honey).unwrap();
            let start = Vec3::new(1.5, 79.25, 1.5);
            let landing = Vec3::new(1.5, 78.9375, 1.5);
            let mut body = PlayerMotion::new(game.motion, start).unwrap();
            body.fall_distance = 10.0;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                    guest.place(start.components());
                    guest.health = 0.5;
                    guest.motion = Some(Box::new(body));
                });
            let entity = game.server.lan.as_ref().unwrap().hub.guests[0].entity_id;
            game.as_guest(1, |game| {
                let moved = game
                    .move_local_player(MoverType::SelfMovement, Vec3::new(0.0, -1.0, 0.0), false)
                    .unwrap();
                assert_eq!(moved, Some(landing.subtract(start)));
                assert_eq!(
                    *trace.lock().unwrap(),
                    [
                        GuestLandingMoment::Death(
                            entity as u32,
                            Some(landing.components()),
                            Some(mod_api::Tag::Double(10.3125_f64.to_bits()))
                        ),
                        GuestLandingMoment::Post(entity),
                    ]
                );
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.health.health(), 5.0);
                assert_eq!(player.fall_distance, 0.0);
            })
            .unwrap();
        });
    });
}

#[test]
#[ignore = "daily"]
fn player_pre_keeps_live_damage_when_resistance_does_not_record_a_reduction() {
    let heard = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225033;
        player.status.resistance = Some(4);
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        assert!(game.hurt_player_from(
            10.0,
            None,
            "minecraft:magic",
            sunrise_gameplay_common::game::creatures::KnownCause::None,
            false
        ));
        assert_eq!(game.player.as_ref().unwrap().health.health(), 19.0);
    });
    assert_eq!(heard.post, [(225033, 1.0)]);
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn the_container_names_what_armor_took() {
    let heard = heard(|game| {
        let mut zombie = mob(game, 225011, "minecraft:zombie");
        // A mob's attack: armor takes its share, where a generic hit bypasses it.
        let attack = Damage::mob(225012, [2.0, 0.0, 2.0]);
        assert!(zombie.damage(
            damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
            10.0,
            attack
        ));
    });
    // A zombie's two armor points against 10 damage: `CombatRules.getDamageAfterAbsorb`
    // clamps 2 - 10 / 2 up to a fifth of the armor, 0.4, and keeps 1 - 0.4 / 25 of the hit.
    let after = 10.0 * (1.0 - 0.4_f32 / 25.0);
    assert_eq!(heard.pre, [(225011, 10.0, after, 10.0 - after)]);
}

/// A level built the normal way lends the game's hooks to its hits: a path that built it
/// without them would run no mod's damage hook.
#[test]
#[ignore = "daily"]
fn a_game_built_level_has_every_damage_hook() {
    with_game(|game| {
        let hooks = &game.damage_hooks;
        assert!(hooks.incoming.is_some() && hooks.pre.is_some() && hooks.post.is_some());
        assert!(hooks.knockback.is_some() && hooks.totem.is_some() && hooks.shield.is_some());
    });
}

#[test]
#[ignore = "daily"]
fn player_melee_hit_preserves_the_attackers_position_and_direct_entity() {
    let heard = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225020;
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        let mut zombie = mob(game, 42, "minecraft:zombie");
        zombie.position = Vec3::new(0.0, 0.0, -2.0);
        game.mobs.push(zombie);
        assert!(game.hurt_player_from(
            8.0,
            Some([0.0, 0.0, -2.0]),
            "minecraft:mob_attack",
            sunrise_gameplay_common::game::creatures::KnownCause::Entity(42),
            false,
        ));
        assert_eq!(
            game.player.as_ref().unwrap().health.health(),
            16.0,
            "a position-based listener changes actual player damage"
        );
    });
    assert_eq!(
        heard.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 225020,
            causing: 42,
            direct: 42,
            direct_type: None,
        })
    );
    assert_eq!(
        heard.positions,
        ["incoming", "pre", "post"].map(|phase| (phase, Some([0.0, 0.0, -2.0])))
    );
}

/// The real projectile hit routes (not hand-built HurtSources) preserve the direct
/// entity through incoming, pre and post, independently of whether there is a shooter.
#[test]
#[ignore = "daily"]
fn arrow_hits_name_the_projectile_on_players_mobs_and_the_dragon() {
    use sunrise_command::args::GameMode;
    use sunrise_sim::health::NO_ATTACK;
    use sunrise_sim::mobs::Who;
    use sunrise_sim::motion::PlayerMotion;
    use sunrise_sim::projectiles::{Hit, Projectile, ProjectileKind};

    for (target, owner, gone, entity, causing) in [
        (Who::Player, Some(Who::Mob(42)), false, 7, 42),
        // AbstractArrow.onHitEntity: an ownerless arrow causes its own hit.
        (Who::Player, None, false, 7, 912),
        (Who::Mob(43), Some(Who::Player), false, 43, 7),
        (Who::Mob(43), None, false, 43, 912),
        (Who::Mob(SPECIAL), Some(Who::Player), false, 44, 7),
        (Who::Mob(43), Some(Who::Mob(42)), true, 43, 42),
    ] {
        let heard = heard(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 7;
            game.player = Some(player);
            game.set_game_mode(GameMode::Survival);
            game.common.mobs.push(mob(game, 42, "minecraft:skeleton"));
            game.mobs[0]
                .extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:stone_sword", 1)));
            *game.held_mut() = Some(ItemStack::new("minecraft:stone_sword", 1));
            game.offhand = Some(ItemStack::new("minecraft:crossbow", 1));
            game.common.mobs.push(mob(game, 43, "minecraft:pig"));
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            // Hover is a sitting phase and correctly shrugs off arrows.
            dragon.set_native_phase(mod_api::DragonPhase::HoldingPattern);
            dragon.id = 44;
            game.dragon.replace(dragon);
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.0, 1.0, 0.5),
                owner,
            );
            arrow.id = 912;
            arrow.arrow.as_mut().unwrap().fired_from_weapon =
                Some(ItemStack::new("minecraft:crossbow", 1));
            arrow.velocity = Vec3::new(0.0, 0.0, -2.0);
            // Equipment changes after launch: only the saved crossbow belongs to the arrow.
            *game.held_mut() = Some(ItemStack::new("minecraft:iron_sword", 1));
            game.mobs[0]
                .extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_sword", 1)));
            if gone {
                game.mobs.retain(|m| m.id != 42);
            }
            game.projectile_hit(
                &arrow,
                Hit::Entity {
                    who: target,
                    at: Vec3::ZERO,
                },
            )
            .unwrap();
            assert_eq!(game.current_attack(), NO_ATTACK, "projectile scope leaked");
            // The listener sets 1 damage: 20 - 1 for a player, 10 - 1 for a pig.
            match target {
                Who::Player => {
                    let health = game.player.as_ref().unwrap().health.health();
                    if owner.is_some() && !gone {
                        assert_eq!(health, 19.0, "the causing-weapon rule changes real damage");
                    } else {
                        assert!(health < 19.0, "ownerless arrows do not use the weapon rule");
                    }
                }
                Who::Mob(SPECIAL) => assert!(game.dragon.as_ref().unwrap().health < 200.0),
                Who::Mob(_) => {
                    if owner.is_some() && !gone {
                        assert_eq!(game.mobs.iter().find(|m| m.id == 43).unwrap().health, 9.0);
                    } else {
                        assert!(game.mobs.iter().find(|m| m.id == 43).unwrap().health < 9.0);
                    }
                }
            }
        });
        let expected = ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity,
            causing,
            direct: 912,
            direct_type: Some("minecraft:arrow".into()),
        });
        assert_eq!(
            heard.sources, expected,
            "target {target:?}, owner {owner:?}"
        );
        assert_eq!(
            heard.weapons,
            [(
                Some("minecraft:crossbow".into()),
                owner
                    .filter(|_| !gone)
                    .map(|_| "minecraft:iron_sword".into())
            )],
            "the saved projectile weapon and the current shooter's main hand are independent"
        );
        if target == Who::Player {
            assert_eq!(
                heard.positions,
                ["incoming", "pre", "post"].map(|phase| (phase, Some([0.0, 1.0, 0.5]))),
                "known projectile position reaches every player hit phase; owner {owner:?}"
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn guest_projectile_uses_current_main_hand_or_none_after_departure() {
    use sunrise_sim::mobs::Who;
    use sunrise_sim::projectiles::{Hit, Projectile, ProjectileKind};

    for gone in [false, true] {
        let log = Arc::new(Mutex::new(Heard::default()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::native_guest_turn_tests::with_guests(|game| {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.selected = 3;
                        guest.inventory[3] = Some(ItemStack::new("minecraft:stone_sword", 1));
                    })
                    .unwrap();
                game.common.mobs.push(mob(game, 43, "minecraft:pig"));
                let mut arrow = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    Vec3::new(0.0, 1.0, 0.5),
                    Some(Who::Player),
                );
                arrow.id = 912;
                arrow.guest = Some(12);
                arrow.arrow.as_mut().unwrap().fired_from_weapon =
                    Some(ItemStack::new("minecraft:crossbow", 1));
                arrow.velocity = Vec3::new(0.0, 0.0, -2.0);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[3] = Some(ItemStack::new("minecraft:iron_sword", 1));
                    })
                    .unwrap();
                if gone {
                    // Model the departed connection being absent when its projectile lands.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .retain(|guest| guest.conn != 12);
                }
                game.projectile_hit(
                    &arrow,
                    Hit::Entity {
                        who: Who::Mob(43),
                        at: Vec3::ZERO,
                    },
                )
                .unwrap();
                let health = game.mobs.iter().find(|m| m.id == 43).unwrap().health;
                if gone {
                    assert!(health < 9.0);
                } else {
                    assert_eq!(health, 9.0, "10 health minus the listener's 1 damage");
                }
            });
        });
        assert_eq!(
            log.lock().unwrap().weapons,
            [(
                Some("minecraft:crossbow".into()),
                (!gone).then(|| "minecraft:iron_sword".into())
            )]
        );
    }
}

/// Live borrowing crosses `living_damage_pre` with no snapshot conversion, preserving
/// recorded reductions and writing `set_new_damage` straight back to the engine container.
#[test]
#[ignore = "daily"]
#[cfg(any())]
fn snapshot_crosses_the_real_damage_hook_with_reductions() {
    let log = heard(|game| {
        let mut container = sunrise_world::living_damage::DamageContainer::new(10.0);
        container.record_reduction(sunrise_world::living_damage::Reduction::Armor, 3.5);
        container.record_new_damage(6.5);
        damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None).damage_pre(
            sunrise_sim::health::Hurting::Player(
                225003,
                HurtSource {
                    damage_type: "minecraft:generic",
                    causing: 0,
                    direct: 0,
                    position: None,
                    direct_type: None,
                },
            ),
            &mut container,
        );
        assert_eq!(container.capture_inflicted(), 2.0);
        assert_eq!(
            container.reduction(sunrise_world::living_damage::Reduction::Armor),
            3.5
        );
    });
    assert_eq!(log.pre, [(225003, 10.0, 2.0, 3.5)]);
}

#[test]
#[ignore = "daily"]
fn a_player_blast_names_its_direct_entity_and_centre() {
    let heard = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225020;
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:creeper")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(42, kind, Vec3::new(10.0, 0.0, 0.0), 0.0));
        game.server.combat.0.explosion_source = Some(42);
        let blast = sunrise_gameplay::game::server_side::explosion::ExplosionSource::default()
            .with_entity(
                42,
                "minecraft:creeper",
                sunrise_sim::health::DirectEntity::Living,
            );
        game.explode(Some(42), [1.0, 0.0, 0.0], 1.5, blast).unwrap();
        game.server.combat.0.explosion_source = None;
        assert!(
            game.player.as_ref().unwrap().health.health() < 20.0,
            "the blast hurts the player"
        );
    });
    assert_eq!(
        heard.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 225020,
            causing: 42,
            direct: 42,
            direct_type: None,
        })
    );
    assert_eq!(
        heard.positions,
        ["incoming", "pre", "post"].map(|phase| (phase, Some([1.0, 0.0, 0.0])))
    );
}

/// The mob branch of the same blast keeps the direct entity and centre it had.
#[test]
#[ignore = "daily"]
fn a_mob_blast_keeps_its_direct_entity_and_centre() {
    let heard = heard(|game| {
        game.player = None;
        let kind = game
            .res
            .mob_types()
            .get("minecraft:creeper")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(42, kind, Vec3::new(10.0, 0.0, 0.0), 0.0));
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs
            .push(Mob::new(43, kind, Vec3::new(0.0, 0.0, 0.0), 0.0));
        game.server.combat.0.explosion_source = Some(42);
        let blast = sunrise_gameplay::game::server_side::explosion::ExplosionSource::default()
            .with_entity(
                42,
                "minecraft:creeper",
                sunrise_sim::health::DirectEntity::Living,
            );
        game.explode(Some(42), [0.0, 0.0, 0.0], 3.0, blast).unwrap();
        game.server.combat.0.explosion_source = None;
        assert!(game.mobs.iter().any(|m| m.id == 43 && m.health < 10.0));
    });
    assert_eq!(
        heard.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 43,
            causing: 42,
            direct: 42,
            direct_type: None,
        })
    );
    assert_eq!(
        heard.positions,
        ["incoming", "pre", "post"].map(|phase| (phase, Some([0.0, 0.0, 0.0])))
    );
}

/// A lingering cloud and a splash potion name themselves as the direct entity, apart from
/// their owner, for mobs and players alike.
#[test]
#[ignore = "daily"]
fn lingering_and_splash_potions_name_themselves_as_the_direct_entity() {
    use sunrise_gameplay_common::game::potions_common::Victim;

    let cloud = heard(|game| {
        game.player = None;
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs.push(Mob::new(43, kind, Vec3::ZERO, 0.0));
        game.cloud_affect(
            Victim::Mob(43),
            &[("instant_damage", 1, 0)],
            (&[], &[], 1.0),
            [0.0, 0.0, 0.0],
            &sunrise_game_data::text::Text::literal("Cloud"),
            (Some(sunrise_sim::mobs::Who::Mob(50)), None),
            Some(900),
        );
    });
    assert_eq!(
        cloud.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 43,
            causing: 50,
            direct: 900,
            direct_type: Some("minecraft:area_effect_cloud".into()),
        })
    );

    let cloud_player = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225020;
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        game.cloud_affect(
            Victim::Player(sunrise_gameplay_common::game::dragon_common::FightWho::Host),
            &[("instant_damage", 1, 0)],
            (&[], &[], 1.0),
            [0.0, 0.0, 0.0],
            &sunrise_game_data::text::Text::literal("Cloud"),
            (Some(sunrise_sim::mobs::Who::Mob(50)), None),
            Some(900),
        );
        assert!(
            game.player.as_ref().unwrap().health.health() < 20.0,
            "the cloud hurts the player"
        );
    });
    assert_eq!(
        cloud_player.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 225020,
            causing: 50,
            direct: 900,
            direct_type: Some("minecraft:area_effect_cloud".into()),
        })
    );
    assert_eq!(
        cloud_player.positions,
        ["incoming", "pre", "post"].map(|phase| (phase, Some([0.0, 0.0, 0.0])))
    );

    let splash = heard(|game| {
        game.player = None;
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs.push(Mob::new(43, kind, Vec3::ZERO, 0.0));
        let mut potion = ItemStack::new("minecraft:splash_potion", 1);
        potion.data_mut().potion = Some("minecraft:harming".into());
        let owner = game.potion_owner(Some(sunrise_sim::mobs::Who::Mob(50)));
        game.splash(
            &potion,
            Vec3::ZERO,
            None,
            None,
            0,
            912,
            &owner,
            true,
            Vec3::ZERO,
        );
    });
    assert_eq!(
        splash.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 43,
            causing: 50,
            direct: 912,
            direct_type: Some("minecraft:splash_potion".into()),
        })
    );

    let splash_player = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225020;
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        let mut potion = ItemStack::new("minecraft:splash_potion", 1);
        potion.data_mut().potion = Some("minecraft:harming".into());
        let owner = game.potion_owner(Some(sunrise_sim::mobs::Who::Mob(50)));
        game.splash(
            &potion,
            Vec3::ZERO,
            None,
            None,
            0,
            912,
            &owner,
            true,
            Vec3::ZERO,
        );
        assert!(
            game.player.as_ref().unwrap().health.health() < 20.0,
            "the splash potion hurts the player"
        );
    });
    assert_eq!(
        splash_player.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 225020,
            causing: 50,
            direct: 912,
            direct_type: Some("minecraft:splash_potion".into()),
        })
    );
    assert_eq!(
        splash_player.positions,
        ["incoming", "pre", "post"].map(|phase| (phase, Some([0.0, 0.0, 0.0])))
    );
}

/// `/damage` and `EntityRef::hurt` keep a causing entity that is not a mob.
#[test]
#[ignore = "daily"]
fn damage_by_a_non_mob_cause_keeps_the_causing_entity() {
    use sunrise_gameplay_common::game::commands_common::Target;

    let heard = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225020;
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 44;
        game.dragon.replace(dragon);
        assert!(game.hurt_target(
            Target::Player,
            2.0,
            "minecraft:generic",
            Some(Target::Dragon(44)),
            Some(Target::Dragon(44)),
            Some([0.0, 0.0, -2.0]),
        ));
    });
    assert_eq!(
        heard.sources,
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 225020,
            causing: 44,
            direct: 44,
            direct_type: Some("minecraft:ender_dragon".into()),
        })
    );
}

/// A wither skull whose wither died or left before impact deals `magic()` with no causing
/// entity, even though the projectile's enclosing attack still holds the dead shooter.
#[test]
#[ignore = "daily"]
fn an_ownerless_wither_skull_has_no_causing_entity() {
    use sunrise_sim::mobs::Who;
    use sunrise_sim::projectiles::{Hit, Projectile, ProjectileKind};

    let heard = heard(|game| {
        let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 225020;
        game.player = Some(player);
        game.set_game_mode(GameMode::Survival);
        // Mob 50 is absent, so `Projectile.getOwner` is none; the skull's own attack
        // still carries the id it was fired with.
        let mut skull = Projectile::new(
            ProjectileKind::WitherSkull { dangerous: false },
            Vec3::ZERO,
            Some(Who::Mob(50)),
        );
        skull.id = 913;
        game.projectile_hit(
            &skull,
            Hit::Entity {
                who: Who::Player,
                at: Vec3::ZERO,
            },
        )
        .unwrap();
        assert!(
            game.player.as_ref().unwrap().health.health() < 20.0,
            "the skull hurts the player"
        );
    });
    // The skull's own `magic()` hit names neither entity; its blast that
    // follows still names no causing entity.
    assert_eq!(
        heard.sources[..3],
        ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity: 225020,
            causing: 0,
            direct: 0,
            direct_type: None,
        })
    );
    assert!(
        heard.sources.iter().all(|hit| hit.causing == 0),
        "no phase names the dead wither: {:?}",
        heard.sources
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6 as _;

use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;
