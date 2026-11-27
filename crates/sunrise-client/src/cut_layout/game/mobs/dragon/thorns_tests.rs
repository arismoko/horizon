//! Exercise the server's attack entry points with real enchantment data.
#![cfg(feature = "client")]
use sunrise_gameplay_common::game::{Arc, ItemStack, Vec3};
use sunrise_sim::{
    mobs::{Damage, Mob, MobEvent, Who, finalize as f},
    projectiles::{Hit, Projectile, ProjectileKind},
};

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::dragon_client::thorns_tests::with_game;
pub(crate) fn thorny() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:diamond_chestplate", 1);
    stack.set_crafting_enchantments(vec![("minecraft:thorns".into(), 255)]);
    stack
}
#[allow(unused_imports)]
pub(crate) use crate::game::client_side::dragon_client::thorns_tests::check_answer;
#[allow(unused_imports)]
pub(crate) use crate::game::client_side::dragon_client::thorns_tests::pair;
#[test]
#[ignore = "daily"]
fn mob_thorns_melee_projectiles_and_non_attacks() {
    with_game(|g| {
        for slot in [f::MAINHAND, f::OFFHAND, f::FEET, f::LEGS, f::CHEST, f::HEAD] {
            let source = pair(g, slot);
            g.handle_mob_events(vec![MobEvent::AttackMob {
                target: 42,
                amount: 1.0,
                source,
                remember_target: true,
            }])
            .unwrap();
            check_answer(g, slot, true);
            assert_eq!(g.mobs[0].health, 96.139_27); // Java Random(1), then the husk's 2 armor.
        }
        for case in 0..5 {
            let source = pair(g, f::CHEST);
            let event = match case {
                0 => MobEvent::AttackMobWithEffect {
                    target: 42,
                    amount: 1.0,
                    source,
                    effect: "minecraft:poison",
                    ticks: 20,
                    amplifier: 0,
                },
                1 => MobEvent::CubeHurtMob {
                    cube: 41,
                    target: 42,
                    amount: 1.0,
                    source,
                },
                2 => MobEvent::IronGolemHit {
                    target: Who::Mob(42),
                    amount: 1.0,
                    source,
                },
                3 => MobEvent::DamageMob {
                    target: 42,
                    amount: 1.0,
                    source,
                },
                _ => {
                    g.mobs[1].invulnerable = 20;
                    g.mobs[1].last_hurt = 10.0;
                    MobEvent::AttackMob {
                        target: 42,
                        amount: 1.0,
                        source,
                        remember_target: true,
                    }
                }
            };
            g.handle_mob_events(vec![event]).unwrap();
            check_answer(g, f::CHEST, case < 3);
        }
        for (kind, answers) in [
            (
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                true,
            ),
            (ProjectileKind::Trident, true),
            (ProjectileKind::SmallFireball, true),
            (ProjectileKind::LargeFireball { power: 0 }, true),
            (ProjectileKind::LlamaSpit, true),
            (
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 1,
                },
                true,
            ),
            (ProjectileKind::WitherSkull { dangerous: false }, true),
            (ProjectileKind::WindCharge, true),
            (ProjectileKind::BreezeWindCharge, true),
            (ProjectileKind::Snowball, false),
            (ProjectileKind::Egg, false),
        ] {
            pair(g, f::CHEST);
            let mut p = Projectile::new(kind, g.mobs[1].position, Some(Who::Mob(41)));
            p.velocity = Vec3::new(-1.0, 0.0, 0.0);
            g.projectile_hit(
                &p,
                Hit::Entity {
                    who: Who::Mob(42),
                    at: p.position,
                },
            )
            .unwrap();
            check_answer(g, f::CHEST, answers);
        }
        pair(g, f::CHEST);
        g.insert_fangs(100, [0.5, 64.0, 0.5], 0.0, -7, 41);
        g.tick_fangs();
        check_answer(g, f::CHEST, true);
    });
}

/// `Guardian.hurtServer`: a still guardian's spikes are 2 of
/// `minecraft:thorns` on the player whose blow it was, dealt inside that hurt
/// (before any tick runs) — not at the end of the tick, where the outbox
/// sweep has no player in place: a serving thread's players are all guests.
#[test]
#[ignore = "daily"]
fn a_still_guardian_spikes_the_player_who_hit_it() {
    use std::sync::{atomic::AtomicI32, mpsc};
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

    with_game(|g| {
        g.lan_host(
            sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "guardian-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        // Keep the channels alive just as for connected guests.
        let mut receivers = Vec::new();
        for (conn, entity) in [(11, 101), (22, 202)] {
            let (tx, rx) = mpsc::channel();
            receivers.push(rx);
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: u128::from(conn),
                    name: format!("GuardianGuest{conn}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                entity,
                [9.5, 64.0, 0.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.load_timeout = 0;
            g.server.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        g.mobs.clear();
        let kind = g.res.mob_types().get("minecraft:guardian").unwrap().clone();
        let mut guardian = Mob::new(41, kind, Vec3::new(10.5, 64.0, 0.5), 0.0);
        // Even a refused hit retaliates, before LivingEntity's guards.
        guardian.invulnerable_flag = true;
        g.mobs.push(guardian);
        let before: Vec<_> = [11, 22]
            .into_iter()
            .map(|conn| {
                g.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(conn)
                    .unwrap()
                    .health
            })
            .collect();
        g.as_guest(11, |g| {
            g.player.as_mut().unwrap().health.advance_clock();
            assert!(!g.hurt_by_player(0, 2.0, Damage::player([9.5, 64.0, 0.5])).0);
            g.player.as_mut().unwrap().health.advance_clock();
            assert_eq!(
                g.player.as_ref().unwrap().health.health(),
                before[0] - 2.0,
                "synchronous thorns must hurt the original attacker (connection 11)"
            );
        })
        .unwrap();
        // The guardian changes targets, and the next player's turn drains the
        // real outbox. A deferred or redirected thorns hit must not reach 22.
        g.mobs[0].ai.target = Some(Who::Mob(202));
        g.as_guest(22, |g| {
            g.player.as_mut().unwrap().health.advance_clock();
            let queued = std::mem::take(&mut g.mobs[0].outbox);
            g.handle_mob_events(queued).unwrap();
            g.player.as_mut().unwrap().health.advance_clock();
        })
        .unwrap();
        for (conn, expected) in [(11, before[0] - 2.0), (22, before[1])] {
            assert_eq!(
                g.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(conn)
                    .unwrap()
                    .health,
                expected,
                "only the original attacking guest takes thorns: connection {conn}"
            );
        }
    });
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
