//! Server-side `living_knockback` tests that construct a server-private `damage_level_of!`
//! damage level directly against `ServerGame` (`with_game`).
use super::native_entity_refs::tests::{bind_local_player, run_fresh_cases, with_game};
use crate::game::ServerGame;
use crate::game::server_side::entity_server::damage_level_of;
use mod_api::{self, Hooks, LevelRandomHost, LivingKnockbackEvent};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_sim::health::NO_ATTACK;
use sunrise_sim::mobs::{Mob, MobEvent, Who};

type Held = (i32, Option<String>, Option<String>);

#[derive(Default)]
struct Heard {
    knocks: Vec<Held>,
    credits: Vec<(i32, Option<String>)>,
    projections: Vec<(Option<bool>, Option<bool>, Option<bool>)>,
    cancel: bool,
    record_projections: bool,
}

struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_knockback(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingKnockbackEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        let id = |s: Option<&mod_api::ItemStackView<'_>>| {
            s.and_then(|s| mod_api::__private::item_key_id(s.item()).map(str::to_owned))
        };
        heard.knocks.push((
            event.attacker,
            id(event.attacker_main.as_ref()),
            id(event.attacker_off.as_ref()),
        ));
        heard
            .credits
            .push((event.kill_credit, id(event.kill_credit_main.as_ref())));
        if heard.record_projections {
            heard.projections.push((
                event.attacker_main.as_ref().map(|s| s.project().is_some()),
                event.attacker_off.as_ref().map(|s| s.project().is_some()),
                event
                    .kill_credit_main
                    .as_ref()
                    .map(|s| s.project().is_some()),
            ));
        }
        event.strength *= 2.0;
        event.x = -event.x;
        if heard.cancel {
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
}

fn heard(test: impl FnOnce(&mut ServerGame<'_>, &Mutex<Heard>)) -> Heard {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(|game| test(game, &log)));
    std::mem::take(&mut *log.lock().unwrap())
}

fn player_with_sword(game: &mut ServerGame<'_>) {
    use sunrise_sim::motion::PlayerMotion;
    let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
    player.entity_id = 7;
    game.player = Some(player);
    game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:iron_sword", 1));
}

fn zombie_at(game: &mut ServerGame<'_>, id: u32, at: Vec3) {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    game.mobs.push(Mob::new(id, kind, at, 0.0));
}

fn with_detached_credit<R>(
    game: &mut ServerGame<'_>,
    victim: i32,
    push: impl FnOnce(&mut sunrise_sim::health::DamageLevel<'_>) -> R,
) -> R {
    let plan = game.prepare_knockback_credit(victim, None, &NO_ATTACK);
    let mut credit = plan.lend(&mut game.server.last_hurt_by_player);
    push(damage_level_of!(game, &NO_ATTACK, &mut credit, None))
}

#[test]
#[ignore = "daily"]
fn a_loaded_sulfur_cube_launch_does_not_reach_the_living_listener() {
    if run_fresh_cases(
        "game::server_side::native_knockback_server_tests::a_loaded_sulfur_cube_launch_does_not_reach_the_living_listener",
        "SUNRISE_SULFUR_CUBE_KNOCKBACK_CHILD",
        &["vanilla_archetypes"],
    ) {
        return;
    }

    let sounds = |m: &Mob| {
        m.outbox
            .iter()
            .filter_map(|e| match e {
                MobEvent::Sound(name, at, volume, pitch) => Some((*name, *at, *volume, *pitch)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let heard = heard(|game, log| {
        game.server.world_root = game.options.game_dir.clone().unwrap();
        game.open_world_datapacks(&[], None).unwrap();
        log.lock().unwrap().cancel = true;
        let kind = game
            .res
            .mob_types()
            .get("minecraft:sulfur_cube")
            .unwrap()
            .clone();
        let hit = sunrise_sim::mobs::Damage::player([1.0, 2.0, 0.0]);
        for (body, resistance_attr, expected_speed, expected_sound) in [
            (
                "minecraft:tnt",
                0.0_f64,
                0.66_f32,
                "entity.sulfur_cube.explosive.hit",
            ),
            (
                "minecraft:iron_sword",
                0.0_f64,
                0.33_f32 * 2.0 * 0.4,
                "entity.sulfur_cube.regular.hit",
            ),
            (
                "minecraft:iron_sword",
                0.9_f64,
                0.33_f32 * 2.0 * ((1.0_f64 - 0.9_f64) as f32) * 0.4,
                "entity.sulfur_cube.regular.hit",
            ),
        ] {
            let mut mob = Mob::new(42, kind.clone(), Vec3::new(0.0, 2.0, 0.0), 0.0);
            mob.extra.set_body(Some(body.into()));
            mob.attrs.knockback_resistance = resistance_attr;
            let aim = sunrise_sim::mobs::Aim {
                eye: [1.0, mob.center()[1], 0.0],
                look: [-1.0, 0.0, 0.0],
                feet: [1.0, 2.0, 0.0],
            };
            with_detached_credit(game, 42, |level| {
                mob.attacker_aim = Some(aim);
                mob.knockback(level, 0.4, 1.0, 0.0, &hit, 4.0);
                assert_eq!(
                    mob.velocity.x,
                    -f64::from(expected_speed),
                    "{body} (r={resistance_attr})"
                );

                mob.velocity = Vec3::ZERO;
                mob.attacker_aim = Some(aim);
                mob.knockback_full(level, 1.0, 1.0, 0.0, &hit, 4.0, true);
                assert_eq!(
                    mob.velocity.x,
                    -f64::from(expected_speed) / 4.0,
                    "{body} (r={resistance_attr})"
                );
                assert_eq!(
                    sounds(&mob),
                    [
                        (expected_sound, [0.0, 2.0, 0.0], 1.0, 1.0),
                        (expected_sound, [0.0, 2.0, 0.0], 1.0, 1.0),
                    ],
                    "{body} (r={resistance_attr})"
                );

                mob.outbox.clear();
                mob.silent = true;
                mob.attacker_aim = Some(aim);
                mob.knockback(level, 0.4, 1.0, 0.0, &hit, 4.0);
                assert!(
                    mob.outbox.is_empty(),
                    "silent cube emitted sound for {body}"
                );
            });
        }
    });
    assert!(heard.knocks.is_empty(), "heard {:?}", heard.knocks);
}

#[test]
#[ignore = "daily"]
fn knockback_fires_and_lends_views_when_stacks_have_unprojectable_components_or_unknown_items() {
    let heard = heard(|game, log| {
        log.lock().unwrap().record_projections = true;
        player_with_sword(game);
        bind_local_player(game, 7, 777);
        zombie_at(game, 42, Vec3::ZERO);
        game.mobs[0].ai.last_hurt_by_player = 100;
        game.remember_mob_player_credit(42);

        let by_player = sunrise_sim::mobs::Damage {
            attacker: Some(Who::Player),
            ..sunrise_sim::mobs::Damage::of(sunrise_sim::mobs::DamageKind::Generic)
        };
        let push = |game: &mut ServerGame<'_>| {
            let attack = game.attack_of(7);
            let plan = game.prepare_knockback_credit(42, None, &attack);
            let mut credit = plan.lend(&mut game.server.last_hurt_by_player);
            let mob = &mut game.common.mobs[0];
            mob.velocity = Vec3::ZERO;
            mob.ground.on_ground = false;
            mob.knockback(
                damage_level_of!(game, &attack, &mut credit, None),
                0.4,
                1.0,
                0.0,
                &by_player,
                0.0,
            );
            mob.velocity
        };

        let doubled_push_velocity = Vec3::new(f64::from(0.8_f32), 0.0, 0.0);
        let mut broken_sword = ItemStack::new("minecraft:iron_sword", 1);
        broken_sword
            .data_mut()
            .set_other("minecraft:damage", Some(sunrise_core::nbt::Tag::Int(-1)));
        assert!(
            sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(&broken_sword)
                .is_err()
        );
        game.common.inventory[game.common.selected] = Some(broken_sword);
        game.offhand = Some(ItemStack::new("minecraft:shield", 1));
        assert_eq!(push(game), doubled_push_velocity);

        log.lock().unwrap().cancel = true;
        assert_eq!(push(game), Vec3::ZERO);
        log.lock().unwrap().cancel = false;

        game.common.inventory[game.common.selected] =
            Some(ItemStack::new("unknown:missing_blade", 1));
        game.offhand = Some(ItemStack::new("minecraft:shield", 1));
        assert_eq!(push(game), doubled_push_velocity);

        game.common.inventory[game.common.selected] =
            Some(ItemStack::new("minecraft:iron_sword", 0));
        game.offhand = Some(ItemStack::new("minecraft:air", 1));
        assert_eq!(push(game), doubled_push_velocity);
    });

    let sword = Some("minecraft:iron_sword".to_string());
    let shield = Some("minecraft:shield".to_string());
    assert_eq!(
        heard.knocks,
        [
            (7, sword.clone(), shield.clone()),
            (7, sword.clone(), shield.clone()),
            (7, None, shield),
            (7, None, None),
        ]
    );
    assert_eq!(
        heard.credits,
        [(7, sword.clone()), (7, sword), (7, None), (7, None),]
    );
    assert_eq!(
        heard.projections,
        [
            (Some(false), Some(true), Some(false)),
            (Some(false), Some(true), Some(false)),
            (None, Some(true), None),
            (None, None, None),
        ]
    );
}
