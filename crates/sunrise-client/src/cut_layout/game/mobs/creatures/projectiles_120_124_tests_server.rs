use crate::game::creatures::projectiles_120_124_tests::PLAIN_FIREWORK_ITEM;
use crate::game::creatures::projectiles_120_124_tests::STAR_FIREWORK_ITEM;
use crate::game::creatures::projectiles_120_124_tests::Target124;
use crate::game::creatures::projectiles_120_124_tests::clear_arena;
use crate::game::creatures::projectiles_120_124_tests::land_positions_match;
use crate::game::creatures::projectiles_120_124_tests::load_witness;
use crate::game::creatures::projectiles_120_124_tests::setup_game;
use crate::game::creatures::projectiles_120_124_tests::vec_bits;
use crate::game::creatures::projectiles_120_124_tests::*;
use std::collections::BTreeMap;
use sunrise_core::random::{LegacyRandom, RandomSource};

pub(crate) fn alive_124(game: &sunrise_gameplay::game::ServerGame<'_>, t: Target124) -> bool {
    match t {
        Target124::Boat(id) => game.boats.iter().any(|b| b.id == id),
        Target124::Cart(id) => game.carts.iter().any(|c| c.id == id),
        Target124::Frame(id) => game.item_frames.iter().any(|f| f.id == id),
        Target124::Painting(id) => game.paintings.iter().any(|p| p.id == id),
        Target124::Falling(id) => game.falling.iter().any(|f| f.id == id),
        Target124::Knot(id) => game.server.others.0.iter().any(|o| o.id == id),
        Target124::Cushion(id) => game.cushions.iter().any(|c| c.id == id),
        Target124::Crystal(id) => game.crystals.iter().any(|c| c.id == id),
        Target124::Tnt(id) => game.tnts.iter().any(|t| t.id == id),
        Target124::Stand(id) => game.armor_stands.iter().any(|a| a.id == id),
    }
}

pub(crate) fn run_witness_firework_rocket_targets_124() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();

    for case in &witness.gap_124_fireworks {
        let label = format!("124 {}", case.case);
        clear_arena(&mut game, 10);
        game.world.random.set_seed(witness.level_seed);

        if case.case == "item_frame" || case.case == "painting" {
            let _ = game.run_command("fill 4 64 2 4 66 6 minecraft:stone");
        }
        if case.case == "leash_knot" {
            let _ = game.run_command("setblock 3 64 4 minecraft:oak_fence");
        }
        if case.case == "armor_stand_blast_nearby" {
            let _ = game.run_command("fill 6 64 3 6 66 5 minecraft:stone");
        }

        let mut targets: BTreeMap<String, Target124> = BTreeMap::new();
        match case.case.as_str() {
            "oak_boat" | "oak_boat_no_star" => {
                assert!(game.run_command("summon minecraft:oak_boat 3.5 64.0 4.5 {}"));
                targets.insert(
                    "boat".into(),
                    Target124::Boat(game.boats.last().unwrap().id),
                );
            }
            "minecart" => {
                assert!(game.run_command("summon minecraft:minecart 3.5 64.0 4.5 {}"));
                targets.insert(
                    "minecart".into(),
                    Target124::Cart(game.carts.last().unwrap().id),
                );
            }
            "item_frame" => {
                assert!(game.run_command(
                    r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
                ));
                targets.insert(
                    "frame".into(),
                    Target124::Frame(game.item_frames.last().unwrap().id),
                );
            }
            "painting" => {
                assert!(game.run_command(
                    r#"summon minecraft:painting 3.5 64.5 4.5 {block_pos:[I;3,64,4],facing:1b,variant:"minecraft:kebab"}"#
                ));
                targets.insert(
                    "painting".into(),
                    Target124::Painting(game.paintings.last().unwrap().id),
                );
            }
            "falling_block" => {
                assert!(game.run_command(
                    r#"summon minecraft:falling_block 3.5 65.0 4.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d]}"#
                ));
                targets.insert(
                    "falling".into(),
                    Target124::Falling(game.falling.last().unwrap().id),
                );
            }
            "leash_knot" => {
                assert!(game.run_command("summon minecraft:leash_knot 3.5 64.375 4.5 {}"));
                targets.insert(
                    "knot".into(),
                    Target124::Knot(game.server.others.0.last().unwrap().id),
                );
            }
            "cushion" => {
                assert!(game.run_command(
                    r#"summon minecraft:cushion 3.5 64.0 4.5 {block_pos:[I;3,64,4],color:"white"}"#
                ));
                targets.insert(
                    "cushion".into(),
                    Target124::Cushion(game.cushions.last().unwrap().id),
                );
            }
            "end_crystal" => {
                assert!(game.run_command("summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                targets.insert(
                    "crystal".into(),
                    Target124::Crystal(game.crystals.last().unwrap().id),
                );
            }
            "tnt" => {
                assert!(game.run_command("summon minecraft:tnt 3.5 64.0 4.5 {fuse:80}"));
                targets.insert("tnt".into(), Target124::Tnt(game.tnts.last().unwrap().id));
            }
            "armor_stand" => {
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                targets.insert(
                    "stand".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
            }
            "armor_stand_marker" => {
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Marker:1b}"));
                targets.insert(
                    "stand".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
            }
            "armor_stand_blast_nearby" => {
                assert!(game.run_command("summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
                targets.insert(
                    "normal".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(
                    game.run_command("summon minecraft:armor_stand 3.5 64.0 6.5 {Invisible:1b}")
                );
                targets.insert(
                    "invisible".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 2.5 {Marker:1b}"));
                targets.insert(
                    "marker".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(game.run_command("summon minecraft:armor_stand 7.5 64.0 4.5 {}"));
                targets.insert(
                    "occluded".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(game.run_command("summon minecraft:armor_stand 9.5 64.0 4.5 {}"));
                targets.insert(
                    "far".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
            }
            other => panic!("unexpected case {other}"),
        }

        let rocket_cmd = if case.case == "armor_stand_blast_nearby" {
            format!(
                "summon minecraft:firework_rocket 3.5 64.5 4.5 {{LifeTime:1,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],FireworksItem:{STAR_FIREWORK_ITEM}}}"
            )
        } else {
            let plain = matches!(
                case.case.as_str(),
                "oak_boat_no_star" | "item_frame" | "painting" | "leash_knot"
            );
            let item = if plain {
                PLAIN_FIREWORK_ITEM
            } else {
                STAR_FIREWORK_ITEM
            };
            format!(
                "summon minecraft:firework_rocket 1.5 {} 4.5 {{LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],FireworksItem:{item}}}",
                case.spawn_y
            )
        };
        assert!(game.run_command(&rocket_cmd), "{rocket_cmd}");
        let idx = game.rockets.len() - 1;
        let rocket_id = game.rockets[idx].id;
        game.rockets.update(idx, |r| {
            r.random = LegacyRandom::new(witness.proj_seed);
        });

        for exp in &case.ticks {
            let (events, sounds) = step_recording(&mut game);
            let live = game.rockets.iter().find(|r| r.id == rocket_id);
            if live.is_some() != exp.rocket_alive {
                failures.push(format!(
                    "{label} t={}: rocket_alive expected {} got {}",
                    exp.tick,
                    exp.rocket_alive,
                    live.is_some()
                ));
                break;
            }
            if let Some(r) = live
                && (vec_bits(r.position) != exp.rocket_pos_bits
                    || vec_bits(r.velocity) != exp.rocket_vel_bits)
            {
                failures.push(format!(
                    "{label} t={}: pos/vel expected {:?}/{:?} got {:?}/{:?}",
                    exp.tick,
                    exp.rocket_pos_bits,
                    exp.rocket_vel_bits,
                    vec_bits(r.position),
                    vec_bits(r.velocity)
                ));
                break;
            }
            let got_alive: BTreeMap<String, bool> = targets
                .iter()
                .map(|(k, &t)| (k.clone(), alive_124(&game, t)))
                .collect();
            if got_alive != exp.targets_alive {
                failures.push(format!(
                    "{label} t={}: targets_alive expected {:?} got {got_alive:?}",
                    exp.tick, exp.targets_alive
                ));
                break;
            }
            let mut got_drops: Vec<String> = game
                .dropped
                .iter()
                .map(|d| d.stack.item.to_string())
                .collect();
            got_drops.sort();
            if got_drops != exp.drops {
                failures.push(format!(
                    "{label} t={}: drops expected {:?} got {got_drops:?}",
                    exp.tick, exp.drops
                ));
                break;
            }
            if let Err(msg) = land_positions_match(&events, &exp.game_events) {
                failures.push(format!("{label} t={}: {msg}", exp.tick));
                break;
            }
            let exp_expl = exp
                .game_events
                .iter()
                .filter(|e| e.id == "minecraft:explode")
                .count();
            let got_expl = events
                .iter()
                .filter(|&&(e, _, _)| e == "minecraft:explode")
                .count();
            if got_expl != exp_expl {
                failures.push(format!(
                    "{label} t={}: explode count expected {exp_expl} got {got_expl}",
                    exp.tick
                ));
                break;
            }
            let exp_break = exp
                .sounds
                .iter()
                .filter(|s| s.as_str() == "minecraft:entity.armor_stand.break")
                .count();
            let got_break = sounds
                .get("minecraft:entity.armor_stand.break")
                .copied()
                .unwrap_or(0);
            if got_break != exp_break {
                failures.push(format!(
                    "{label} t={}: armor_stand.break count expected {exp_break} got {got_break}",
                    exp.tick
                ));
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "gap #124 failures:\n{}",
        failures.join("\n")
    );
}

pub(crate) fn clear_others(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.server.others.0.clear();
}
