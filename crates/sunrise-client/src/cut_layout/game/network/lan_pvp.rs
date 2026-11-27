#![cfg(test)]
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Who;
#[cfg(test)]
pub(crate) mod arrow_hit_sound_tests {
    use std::fmt::Write as _;
    use std::path::PathBuf;
    use std::sync::{Arc, atomic::AtomicUsize, mpsc};
    use sunrise_command::args::GameMode;
    use sunrise_core::geometry::Vec3;
    use sunrise_game_data::gamerules::Value;
    use sunrise_protocol::packets::play::Clientbound;
    use sunrise_sim::{
        lan::{Out, PlayerSender},
        motion::PlayerMotion,
        projectiles::{Hit, Projectile, ProjectileKind},
    };
    use {sunrise_gameplay::game::server_side::lan_pvp::Victim, sunrise_sim::mobs::Who};

    fn load_witness() -> Witness {
        let value = std::env::var_os("SUNRISE_ARROW_HIT_DING_WITNESS").unwrap_or_else(|| {
            panic!(
                "SUNRISE_ARROW_HIT_DING_WITNESS is unset; run tools/test_arrow_hit_ding_parity.py, which records the witness"
            )
        });
        let raw_path = PathBuf::from(value);
        let path = if raw_path.is_dir() {
            raw_path.join("arrow_hit_ding.json")
        } else {
            raw_path
        };
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "failed to read witness {} (record via tools/test_arrow_hit_ding_parity.py): {e}",
                path.display()
            )
        });
        serde_json::from_str(&raw).expect("valid 26.3 arrow-hit witness JSON")
    }

    #[derive(serde::Deserialize)]
    struct Witness {
        minecraft_version: String,
        cases: Vec<WitnessCase>,
    }

    #[derive(serde::Deserialize)]
    struct WitnessCase {
        name: String,
        shooter: String,
        victim: String,
        silent: bool,
        pvp: bool,
        victim_mode: String,
        victim_hurt: bool,
        victim_health_before_bits: String,
        victim_health_after_bits: String,
        alice_game_events: Vec<WireGameEvent>,
        bob_game_events: Vec<WireGameEvent>,
    }

    #[derive(Debug, PartialEq, Eq, serde::Deserialize)]
    struct WireGameEvent {
        event: u8,
        param_bits: String,
        payload_hex: String,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CaseOutcome {
        victim_hurt: bool,
        victim_health_before_bits: String,
        victim_health_after_bits: String,
        alice_game_events: Vec<WireGameEvent>,
        bob_game_events: Vec<WireGameEvent>,
    }

    fn hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            write!(out, "{b:02x}").unwrap();
        }
        out
    }

    fn float_bits_hex(value: f32) -> String {
        format!("{:08x}", value.to_bits())
    }

    fn player_conn(name: &str) -> u32 {
        match name {
            "Alice" => 12,
            "Bob" => 13,
            other => panic!("unexpected witness player: {other}"),
        }
    }

    /// Wire `ClientboundGameEventPacket` records sent to a guest connection.
    fn game_events(receiver: &mpsc::Receiver<Out>) -> Vec<WireGameEvent> {
        let mut events = Vec::new();
        for sent in receiver.try_iter() {
            let bodies = match sent {
                Out::Packet(p) => vec![p],
                Out::Packets(p) => p,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            };
            for body in bodies {
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                if let Ok(Clientbound::GameEvent(e)) = Clientbound::decode(id, reader.rest()) {
                    events.push(WireGameEvent {
                        event: e.event,
                        param_bits: float_bits_hex(e.param),
                        payload_hex: hex(&body),
                    });
                }
            }
        }
        events
    }

    /// Run one recorded vanilla 26.3 arrow-hit case between guests 12 (Alice) and 13 (Bob).
    fn run_case(case: &WitnessCase) -> CaseOutcome {
        let shooter = player_conn(&case.shooter);
        let victim = player_conn(&case.victim);
        let victim_mode = match case.victim_mode.as_str() {
            "survival" => GameMode::Survival,
            "creative" => GameMode::Creative,
            other => panic!("unexpected witness victim_mode: {other}"),
        };
        let mut outcome = None;
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let mut receivers = Vec::new();
            for conn in [12, 13] {
                let (tx, rx) = mpsc::channel();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| {
                        g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
                    })
                    .unwrap();
                receivers.push(rx);
            }
            let mut body = PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0)).unwrap();
            body.entity_id = victim as i32;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(victim, |g| {
                    g.motion = Some(Box::new(body));
                    g.load_timeout = 0;
                })
                .unwrap();
            assert!(game.ui.rules.set("pvp", Value::Bool(case.pvp)));
            game.as_guest(victim, |g| {
                g.set_game_mode(GameMode::Creative);
                if victim_mode != GameMode::Creative {
                    g.set_game_mode(victim_mode);
                }
            })
            .unwrap();
            for g in &game.server.lan.as_ref().unwrap().hub.guests {
                g.sender.flush();
            }
            for rx in &receivers {
                while rx.try_recv().is_ok() {}
            }
            let health_before = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(victim)
                .unwrap()
                .health;
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(1.0, 65.0, 5.0),
                Some(Who::Player),
            );
            arrow.guest = Some(shooter);
            arrow.velocity = Vec3::new(0.0, -1.0, 0.0);
            if case.silent {
                game.persist.raw.insert(
                    arrow.id,
                    sunrise_core::nbt::Tag::compound([("Silent", sunrise_core::nbt::Tag::Byte(1))]),
                );
            }
            let hit = Hit::Entity {
                who: Who::Player,
                at: Vec3::new(2.7, 65.0, 5.0),
            };
            game.projectile_hits_player(&arrow, hit, Victim::Guest(victim))
                .unwrap();
            let health_after = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(victim)
                .unwrap()
                .health;
            for g in &game.server.lan.as_ref().unwrap().hub.guests {
                g.sender.flush();
            }
            outcome = Some(CaseOutcome {
                victim_hurt: health_after < health_before,
                victim_health_before_bits: float_bits_hex(health_before),
                victim_health_after_bits: float_bits_hex(health_after),
                alice_game_events: game_events(&receivers[0]),
                bob_game_events: game_events(&receivers[1]),
            });
        });
        outcome.unwrap()
    }

    /// Vanilla 26.3 witness recorded by `tools/test_arrow_hit_ding_parity.py`: exact wire `ClientboundGameEventPacket` and victim health bits on player arrow hits.
    #[test]
    #[ignore = "run by tools/test_arrow_hit_ding_parity.py"]
    fn an_arrow_that_hurts_another_player_dings_its_shooter() {
        run_an_arrow_that_hurts_another_player_dings_its_shooter();
    }

    pub(crate) fn run_an_arrow_that_hurts_another_player_dings_its_shooter() {
        let witness = load_witness();
        assert_eq!(witness.minecraft_version, "26.3");
        assert!(!witness.cases.is_empty());
        for case in witness.cases {
            let actual = run_case(&case);
            let expected = CaseOutcome {
                victim_hurt: case.victim_hurt,
                victim_health_before_bits: case.victim_health_before_bits,
                victim_health_after_bits: case.victim_health_after_bits,
                alice_game_events: case.alice_game_events,
                bob_game_events: case.bob_game_events,
            };
            assert_eq!(actual, expected, "26.3 witness case {}", case.name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_command::args::GameMode;
    use sunrise_game_data::gamerules::Value;
    use sunrise_gameplay::game::native_entity_refs::tests::{
        bind_local_player, fixture_resident_column, run_fresh_cases, run_with_game_stack,
    };
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use sunrise_gameplay::game::server_side::lan_pvp::{Strike, Sweep, Victim};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::{
        mobs::Mob,
        motion::PlayerMotion,
        projectiles::{Hit, Projectile, ProjectileKind},
    };

    fn ready_victim(game: &mut sunrise_gameplay::game::ServerGame<'_>, with_full_armor: bool) {
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |guest| {
                guest.loaded = true;
                guest.load_timeout = 0;
            })
            .unwrap();
        game.as_guest(13, |game| {
            game.set_mode(GameMode::Survival);
            let mut chest = ItemStack::new("minecraft:diamond_chestplate", 1);
            chest.set_crafting_enchantments(vec![("minecraft:thorns".into(), 255)]);
            game.armor[1] = Some(chest);
            if with_full_armor {
                game.armor[0] = Some(ItemStack::new("minecraft:diamond_helmet", 1));
                game.armor[2] = Some(ItemStack::new("minecraft:diamond_leggings", 1));
                game.armor[3] = Some(ItemStack::new("minecraft:diamond_boots", 1));
            }
            game.put_equipment_modifiers();
        })
        .unwrap();
    }

    /// A fresh survival body for guest `conn` (entity `conn`), where `with_guests` stands it.
    fn fresh_body(game: &mut sunrise_gameplay::game::ServerGame<'_>, conn: u32) {
        let mut body =
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0)).unwrap();
        body.entity_id = conn as i32;
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(conn, |g| {
                g.motion = Some(Box::new(body));
                g.load_timeout = 0;
                g.loaded = true;
            })
            .unwrap();
        game.as_guest(conn, |g| {
            g.set_game_mode(GameMode::Creative);
            g.set_game_mode(GameMode::Survival);
        })
        .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn pvp_mace_smash_keeps_the_players_breach_and_does_not_retaliate_on_a_bystander() {
        if run_fresh_cases(
            "cut_layout::game::server_side::lan_pvp::tests::pvp_mace_smash_keeps_the_players_breach_and_does_not_retaliate_on_a_bystander",
            "SUNRISE_PVP_MACE_SMASH_CHILD",
            &["smash"],
        ) {
            return;
        }
        run_with_game_stack(|| {
            with_guests(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                ready_victim(game, true);
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Hard;
                let mut mace = ItemStack::new("minecraft:mace", 1);
                mace.enchant("minecraft:breach", 4);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[guest.selected] = Some(mace);
                    })
                    .unwrap();
                let mut strike = Strike::test_blow(12);
                strike.amount = 5.0;
                strike.damage_type = "minecraft:mace_smash".into();
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs
                    .push(Mob::new(41, cow, Vec3::new(3.0, 64.0, 5.0), 0.0));
                let bystander_health = game.mobs[0].health;
                assert!(game.pvp_land(Victim::Guest(13), &strike).is_some());
                let victim = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
                // CombatRules.getDamageAfterAbsorb: full diamond blocks 0.75 of a 5-point
                // hit; Breach IV subtracts 0.60 from that fraction, leaving 4.25 damage.
                assert_eq!(victim.health, 15.75);
                assert_eq!(game.mobs[0].health, bystander_health);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn guest_thorns_answers_the_host_that_shot_the_arrow() {
        if run_fresh_cases(
            "cut_layout::game::server_side::lan_pvp::tests::guest_thorns_answers_the_host_that_shot_the_arrow",
            "SUNRISE_PVP_ARROW_THORNS_CHILD",
            &["arrow"],
        ) {
            return;
        }
        run_with_game_stack(|| {
            with_guests(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                ready_victim(game, false);
                let start = Vec3::new(1.0, 65.0, 5.0);
                game.player = Some(PlayerMotion::new(game.motion, start).unwrap());
                bind_local_player(game, 77, 77);
                game.set_mode(GameMode::Survival);
                let before = game.player.as_ref().unwrap().health.health();
                let at = Vec3::new(3.0, 65.0, 5.0);
                let mut arrow = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    at,
                    Some(Who::Player),
                );
                arrow.id = 90;
                arrow.previous = start;
                arrow.velocity = Vec3::new(2.0, 0.0, 0.0);
                game.projectile_hits_player(
                    &arrow,
                    Hit::Entity {
                        who: Who::Mob(13),
                        at,
                    },
                    Victim::Guest(13),
                )
                .unwrap();
                let victim = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
                assert!(victim.health < 20.0, "the arrow did not hurt the guest");
                let host = game.player.as_ref().unwrap();
                assert_eq!(host.entity_id, 77);
                assert!(
                    host.health.health() < before,
                    "Thorns did not hurt the shooter"
                );
            });
        });
    }

    fn health(game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> f32 {
        let host = game.server.lan.as_ref().unwrap();
        host.hub
            .guest(conn)
            .unwrap()
            .motion
            .as_ref()
            .unwrap()
            .health
            .health()
    }

    /// What each player-on-player rule answers for guest 12 on guest 13, each from a fresh
    /// body: the `PlayerRef::can_harm` read, an arrow (`AbstractArrow.canHitEntity`), another
    /// projectile or a trident (the victim's hurt), a spear's stab, a melee blow, its sweep
    /// (which also spares allies, `doSweepAttack`'s `isAlliedTo`) and a blast of guest 12's
    /// primed TNT.
    fn every_site(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> [bool; 7] {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        fresh_body(game, 13);
        let read = a.can_harm(&*game, b) == Some(true);
        let arrow = game.can_arrow_harm_player(Who::Mob(12), None, Who::Mob(13));
        let projectile = game.guest_may_harm(Some(12), 13);
        let spear = game.stab_players_for(12).iter().any(|p| p.conn == 13);
        let attacker_team = game.score.board.players_team("guest12").cloned();
        let mut blow = Strike::test_blow(12);
        blow.team = attacker_team.clone();
        let melee = game.pvp_land(Victim::Guest(13), &blow).is_some();

        fresh_body(game, 13);
        let sweep = Sweep {
            attacker: Some(12),
            creative: false,
            from: [3.0, 64.0, 5.0],
            damage: 1.0,
            enchanted_damage: None,
            knockback: (0.0, 0.0),
            team: attacker_team,
            hit_box: Aabb::new(Vec3::new(0.0, 60.0, 0.0), Vec3::new(6.0, 70.0, 10.0)),
            skip: None,
        };
        let before = health(game, 13);
        game.sweep_players(&sweep);
        let swept = health(game, 13) < before;

        fresh_body(game, 13);
        game.server.combat.0.blast_owner =
            Some((12, sunrise_game_data::text::Text::literal("guest12")));
        game.server.combat.0.blast_owner_player = true;
        game.server.combat.0.blast_guest = Some(12);
        let before = health(game, 13);
        game.server_explode(
            [3.0, 64.0, 8.0],
            2.0,
            sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
            false,
        )
        .unwrap();
        let blasted = health(game, 13) < before;
        game.server.combat.0.blast_owner = None;
        game.server.combat.0.blast_owner_player = false;
        game.server.combat.0.blast_guest = None;
        [read, arrow, projectile, spear, melee, swept, blasted]
    }

    /// The guests of `with_guests`, in an empty chunk the blasts see through.
    fn with_two_guests(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            test(game);
        });
    }

    /// `ServerPlayer.canHarmPlayer` is answered in one place,
    /// [`can_harm_player`](sunrise_client::health::can_harm_player), and every
    /// player-on-player rule takes its answer for guest 12 on guest 13, the `pvp` rule first,
    /// then the teams. The sweep alone also spares allies.
    #[test]
    #[ignore = "daily"]
    fn every_player_on_player_rule_takes_can_harm_player() {
        with_two_guests(|game| {
            let mut answers = Vec::new();
            for (pvp, team) in [
                (true, None),
                (false, None),
                (true, Some(false)),
                (true, Some(true)),
            ] {
                game.ui.rules.set("pvp", Value::Bool(pvp));
                if let Some(friendly_fire) = team {
                    let board = &mut game.score.board;
                    board.add_team("red").allow_friendly_fire = friendly_fire;
                    board.add_to_team("guest12", "red");
                    board.add_to_team("guest13", "red");
                }
                let board = &game.score.board;
                let (own, other) = (board.players_team("guest12"), board.players_team("guest13"));
                let allied = sunrise_game_data::scoreboard::teams_allied(own, other);
                let expected = sunrise_sim::health::can_harm_player(12, 13, pvp, own, other);
                answers.push(expected);
                let mut wanted = [expected; 7];
                wanted[5] = expected && !allied;
                assert_eq!(every_site(game), wanted, "pvp {pvp}, team {team:?}");
            }
            assert_eq!(answers, [true, false, false, true]);
        });
    }

    /// A mod's row that changes `canHarmPlayer`'s answer changes it at every site: with the
    /// answer replaced on this thread (`rule_replacements::HARM`), two players on a server with
    /// `pvp` off hurt each other everywhere, and with `pvp` on and no teams nowhere.
    #[test]
    #[ignore = "daily"]
    fn a_replaced_can_harm_player_answer_reaches_every_site() {
        use sunrise_gameplay_common::game::rule_replacements::{HARM, replaced};
        with_two_guests(|game| {
            game.ui.rules.set("pvp", Value::Bool(false));
            let sites = replaced(&HARM, |_, _| true, || every_site(game));
            assert_eq!(sites, [true; 7], "pvp off, answered true");
            game.ui.rules.set("pvp", Value::Bool(true));
            let sites = replaced(&HARM, |_, _| false, || every_site(game));
            assert_eq!(sites, [false; 7], "pvp on, answered false");
        });
    }
}
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
