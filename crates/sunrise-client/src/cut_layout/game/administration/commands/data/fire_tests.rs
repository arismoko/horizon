use std::collections::HashSet;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};
use {
    sunrise_gameplay::game::server_side::commands::data::DataRef,
    sunrise_gameplay_common::game::commands_common::Target,
};

pub(crate) fn read_witness() -> String {
    let raw = std::env::var_os("SUNRISE_ENTITY_FIRE_TICKS_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_ENTITY_FIRE_TICKS_WITNESS is unset; run through tools/test_entity_fire_ticks_parity.py, which records the 26.3 witness"
        )
    });
    let path = std::path::PathBuf::from(raw);
    let file = if path.is_dir() {
        path.join("entity_fire_ticks.tsv")
    } else {
        path
    };
    std::fs::read_to_string(&file).unwrap_or_else(|err| {
        panic!(
            "failed to read {} (run through tools/test_entity_fire_ticks_parity.py): {err}",
            file.display()
        )
    })
}

pub(crate) struct WitnessRow<'a> {
    pub(crate) kind: &'a str,
    pub(crate) initial_fire: &'a str,
    pub(crate) nbt: &'a str,
    pub(crate) ticks: usize,
    pub(crate) expected: Option<Tag>,
}

pub(crate) struct Active<'a> {
    pub(crate) kind: &'a str,
    pub(crate) initial_fire: &'a str,
    pub(crate) nbt: &'a str,
    pub(crate) target: Target,
    pub(crate) elapsed: usize,
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::data::fire_tests::target_exists;

pub(crate) fn summon_target_at(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    kind: &str,
    pos: [f64; 3],
    nbt: &str,
) -> Target {
    let returned = game.summon_with_nbt(kind, pos, nbt).unwrap();
    match kind {
        "minecraft:evoker_fangs" => Target::Fangs(game.fangs.list.last().unwrap().id),
        "minecraft:area_effect_cloud" => Target::Cloud(game.breath_clouds.last().unwrap().id),
        _ => returned.unwrap_or_else(|| panic!("missing summon target for {kind}")),
    }
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::data::fire_tests::tick_fire_entities;

pub(crate) fn parse_witness_26_3(witness: &str) -> Vec<WitnessRow<'_>> {
    assert!(
        witness.starts_with("# vanilla 26.3 witness "),
        "witness must be tagged with vanilla 26.3"
    );
    let mut lines = witness
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    assert_eq!(
        lines.next(),
        Some("kind\tinitial_fire\tnbt\tticks\tstate\tfire")
    );
    let rows: Vec<WitnessRow<'_>> = lines
        .map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            assert_eq!(parts.len(), 6, "malformed 26.3 witness row: {line}");
            let expected = match (parts[4], parts[5]) {
                ("alive", raw) => Some(Tag::Short(raw.parse().unwrap())),
                ("gone", "gone") => None,
                other => panic!("unexpected 26.3 witness state: {other:?}"),
            };
            WitnessRow {
                kind: parts[0],
                initial_fire: parts[1],
                nbt: parts[2],
                ticks: parts[3].parse().unwrap(),
                expected,
            }
        })
        .collect();
    assert!(!rows.is_empty());
    rows
}

/// Vanilla 26.3 witness parity (#2683): compare Fire ticks, numeric tag decoding,
/// save/reload, swept fire contact, lava first-tick gating, silence/order, and
/// tracked fire flags against the 26.3 oracle output recorded by
/// `tools/test_entity_fire_ticks_parity.py`.
#[test]
#[ignore = "run by tools/test_entity_fire_ticks_parity.py"]
fn fire_changes_on_eyes_fangs_spawners_clouds_and_bullets() {
    run_fire_changes_on_eyes_fangs_spawners_clouds_and_bullets();
}

pub(crate) fn run_fire_changes_on_eyes_fangs_spawners_clouds_and_bullets() {
    let witness = read_witness();
    let rows = parse_witness_26_3(&witness);
    let total_rows = rows.len();
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(20, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0 15 15"));
        for command in [
            "fill 9 219 7 15 219 9 minecraft:stone",
            "setblock 10 219 8 minecraft:netherrack",
            "setblock 10 220 8 minecraft:fire",
            "fill 13 220 7 15 220 9 minecraft:stone",
            "setblock 14 220 8 minecraft:lava",
        ] {
            assert!(game.run_command(command), "{command}");
        }

        let mut active: Option<Active<'_>> = None;
        let mut checked = 0usize;
        for row in rows {
            let same_case = matches!(
                &active,
                Some(a)
                    if a.kind == row.kind
                        && a.initial_fire == row.initial_fire
                        && a.nbt == row.nbt
                        && row.ticks >= a.elapsed
            );
            if !same_case {
                if let Some(prev) = active.take()
                    && target_exists(game, prev.target)
                {
                    game.kill_target(prev.target);
                }
                let target =
                    match row.initial_fire {
                        "bullet_swept_fire_dry_end" => summon_target_at(
                            game,
                            row.kind,
                            [8.5, 220.2, 8.5],
                            "{NoGravity:1b,Fire:0s,Motion:[3.5d,0.0d,0.0d]}",
                        ),
                        "bullet_fresh_in_lava_40" => summon_target_at(
                            game,
                            row.kind,
                            [14.5, 220.2, 8.5],
                            "{NoGravity:1b,Fire:40s,HasBeenShot:1b}",
                        ),
                        "bullet_merge_fire_40_air" => {
                            let t = summon_target_at(
                                game,
                                row.kind,
                                [8.5, 220.2, 8.5],
                                "{UUID:[I;2683,1,0,3],NoGravity:1b,Fire:0s}",
                            );
                            tick_fire_entities(game);
                            assert!(game.run_command(
                                "data merge entity 00000a7b-0000-0001-0000-000000000003 {Fire:40s}"
                            ));
                            t
                        }
                        "bullet_ticked_in_lava_40" => {
                            let t = summon_target_at(
                                game,
                                row.kind,
                                [8.5, 220.2, 8.5],
                                "{UUID:[I;2683,1,0,4],NoGravity:1b,Fire:0s}",
                            );
                            tick_fire_entities(game);
                            assert!(game.run_command(
                                "tp 00000a7b-0000-0001-0000-000000000004 14.5 220.2 8.5"
                            ));
                            assert!(game.run_command(
                                "data merge entity 00000a7b-0000-0001-0000-000000000004 {Fire:40s}"
                            ));
                            t
                        }
                        _ => summon_target_at(game, row.kind, [8.5, 220.0, 8.5], row.nbt),
                    };
                active = Some(Active {
                    kind: row.kind,
                    initial_fire: row.initial_fire,
                    nbt: row.nbt,
                    target,
                    elapsed: 0,
                });
            }
            let current = active.as_mut().unwrap();
            while current.elapsed < row.ticks {
                tick_fire_entities(game);
                current.elapsed += 1;
            }
            match row.expected {
                Some(expected_tag) => {
                    assert!(
                        target_exists(game, current.target),
                        "expected {} {} tick={} to remain alive",
                        row.kind,
                        row.initial_fire,
                        row.ticks
                    );
                    let live = game
                        .data_of(&DataRef::Entity(current.target))
                        .unwrap_or_else(|e| {
                            panic!(
                                "data_of failed for alive {} {} tick={}: {e:?}",
                                row.kind, row.initial_fire, row.ticks
                            )
                        });
                    assert_eq!(
                        live.get("Fire"),
                        Some(&expected_tag),
                        "vanilla 26.3 live Fire mismatch for {} {} tick={}",
                        row.kind,
                        row.initial_fire,
                        row.ticks
                    );
                }
                None => {
                    assert!(
                        !target_exists(game, current.target),
                        "expected {} {} tick={} to be removed",
                        row.kind,
                        row.initial_fire,
                        row.ticks
                    );
                }
            }
            checked += 1;
        }
        if let Some(prev) = active.take()
            && target_exists(game, prev.target)
        {
            game.kill_target(prev.target);
        }
        assert_eq!(
            checked, total_rows,
            "every 26.3 witness row must be checked"
        );

        // Chunk save & reload parity: a surviving bullet loaded with Fire:-5s resets to 0s
        // both in /data get and in the chunk writer (`save_projectile` must not let raw overwrite Fire).
        let neg_bullet = summon_target_at(
            game,
            "minecraft:shulker_bullet",
            [8.5, 220.2, 8.5],
            "{NoGravity:1b,Fire:-5s}",
        );
        tick_fire_entities(game);
        let Target::Projectile(neg_id) = neg_bullet else {
            unreachable!();
        };
        let live_neg = game.data_of(&DataRef::Entity(neg_bullet)).unwrap();
        assert_eq!(live_neg.get("Fire"), Some(&Tag::Short(0)));
        let idx = game
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == neg_id)
            .unwrap();
        let keys: HashSet<(i32, i32)> = HashSet::from([(0, 0)]);
        let (_, saved_bullet) = game.save_projectile(idx, &keys).unwrap();
        assert_eq!(
            saved_bullet.get("Fire"),
            Some(&Tag::Short(0)),
            "chunk save must write live Fire:0s, not raw -5s"
        );
        let reloaded = sunrise_sim::entity_nbt::spit_or_bullet_from_nbt(&saved_bullet, None, None)
            .expect("saved bullet reloads");
        assert_eq!(reloaded.fire_ticks, 0);
        game.kill_target(neg_bullet);

        // Fresh vs already-ticked bullet in lava, plus shoot -> hurt -> discard -> entity_damage order and Silent:1b.
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| *e.borrow_mut() = Some(Vec::new()));
        game.server.audio_capture_active = true;
        game.server.pending_sounds.clear();
        let fresh = summon_target_at(
            game,
            "minecraft:shulker_bullet",
            [14.5, 220.2, 8.5],
            "{NoGravity:1b,Fire:40s}",
        );
        tick_fire_entities(game);
        assert!(!target_exists(game, fresh));
        let fresh_events: Vec<&'static str> =
            sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|e| {
                std::mem::take(e.borrow_mut().as_mut().unwrap())
                    .into_iter()
                    .map(|(ev, ..)| ev)
                    .collect()
            });
        let fresh_sounds: Vec<String> = std::mem::take(&mut game.server.pending_sounds)
            .into_iter()
            .map(|s| s.event)
            .collect();
        let shoot_pos = fresh_events
            .iter()
            .position(|&e| e == "minecraft:projectile_shoot")
            .expect("PROJECTILE_SHOOT must fire before onFire removal");
        let dmg_pos = fresh_events
            .iter()
            .position(|&e| e == "minecraft:entity_damage")
            .expect("ENTITY_DAMAGE must fire on onFire removal");
        assert!(
            shoot_pos < dmg_pos,
            "PROJECTILE_SHOOT must precede ENTITY_DAMAGE: {fresh_events:?}"
        );
        assert!(
            fresh_sounds
                .iter()
                .any(|s| s == "minecraft:entity.shulker_bullet.hurt"),
            "fresh lava Fire:40s bullet plays hurt sound: {fresh_sounds:?}"
        );
        assert!(
            !fresh_sounds
                .iter()
                .any(|s| s == "minecraft:entity.generic.burn"),
            "fresh lava Fire:40s bullet dies to onFire (firstTick gates isInLava), not lavaHurt: {fresh_sounds:?}"
        );

        // Already-ticked bullet moved into lava with Fire:40s is exempt from onFire and dies to lavaHurt (plays generic.burn).
        let ticked = summon_target_at(
            game,
            "minecraft:shulker_bullet",
            [8.5, 220.2, 8.5],
            "{UUID:[I;2683,2,0,1],NoGravity:1b,Fire:0s}",
        );
        tick_fire_entities(game);
        assert!(game.run_command("tp 00000a7b-0000-0002-0000-000000000001 14.5 220.2 8.5"));
        assert!(
            game.run_command("data merge entity 00000a7b-0000-0002-0000-000000000001 {Fire:40s}")
        );
        game.server.pending_sounds.clear();
        tick_fire_entities(game);
        assert!(!target_exists(game, ticked));
        let ticked_sounds: Vec<String> = std::mem::take(&mut game.server.pending_sounds)
            .into_iter()
            .map(|s| s.event)
            .collect();
        assert!(
            ticked_sounds
                .iter()
                .any(|s| s == "minecraft:entity.shulker_bullet.hurt")
                && ticked_sounds
                    .iter()
                    .any(|s| s == "minecraft:entity.generic.burn"),
            "already-ticked lava Fire:40s bullet dies to lavaHurt with burn sound: {ticked_sounds:?}"
        );

        // Silent:1b suppresses shulker_bullet.hurt on both fire and attack paths.
        game.server.pending_sounds.clear();
        let silent_fire = summon_target_at(
            game,
            "minecraft:shulker_bullet",
            [8.5, 220.2, 8.5],
            "{NoGravity:1b,Fire:40s,Silent:1b}",
        );
        tick_fire_entities(game);
        assert!(!target_exists(game, silent_fire));
        assert!(
            std::mem::take(&mut game.server.pending_sounds).is_empty(),
            "Silent:1b bullet must not play hurt sound on fire death"
        );

        game.server.pending_sounds.clear();
        let silent_attack = summon_target_at(
            game,
            "minecraft:shulker_bullet",
            [8.5, 220.2, 8.5],
            "{NoGravity:1b,Fire:0s,Silent:1b}",
        );
        let Target::Projectile(sa_id) = silent_attack else {
            unreachable!();
        };
        let sa_idx = game
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == sa_id)
            .unwrap();
        let _ = game.destroy_shulker_bullet(sa_idx);
        assert!(!target_exists(game, silent_attack));
        assert!(
            std::mem::take(&mut game.server.pending_sounds).is_empty(),
            "Silent:1b bullet must not play hurt sound on attack path"
        );
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| *e.borrow_mut() = None);

        // ShulkerBullet.isOnFire() is always false, even with positive fire_ticks or visual fire.
        let bullet_vis = summon_target_at(
            game,
            "minecraft:shulker_bullet",
            [8.5, 220.2, 8.5],
            "{NoGravity:1b,Fire:15s,HasVisualFire:1b}",
        );
        let Target::Projectile(bv_id) = bullet_vis else {
            unreachable!();
        };
        assert!(
            !game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == bv_id)
                .unwrap()
                .on_fire(),
            "shulker bullets must never render flames"
        );
        game.kill_target(bullet_vis);

        // Non-entity-ticking ominous item spawner must not tick Fire down.
        assert!(!game.entity_ticking_area()(328, 8));
        let far_spawner = summon_target_at(
            game,
            "minecraft:ominous_item_spawner",
            [328.5, 220.0, 8.5],
            "{Fire:40s,spawn_item_after_ticks:100L,item:{id:\"minecraft:stone\",count:1}}",
        );
        game.tick_ominous_item_spawners();
        assert_eq!(
            game.data_of(&DataRef::Entity(far_spawner))
                .unwrap()
                .get("Fire"),
            Some(&Tag::Short(40)),
            "spawner outside entity-ticking area must not decrement Fire"
        );
        game.kill_target(far_spawner);

        // Tracked on-fire bit transitions (burning -> clear) and independent HasVisualFire
        // on fangs, summoned eyes, and ominous item spawners.
        let fang = summon_target_at(
            game,
            "minecraft:evoker_fangs",
            [8.5, 220.0, 8.5],
            "{Fire:2s,Warmup:100}",
        );
        let eye = summon_target_at(
            game,
            "minecraft:eye_of_ender",
            [8.5, 220.0, 8.5],
            "{Fire:2s}",
        );
        let spawner = summon_target_at(
            game,
            "minecraft:ominous_item_spawner",
            [8.5, 220.0, 8.5],
            "{Fire:2s,spawn_item_after_ticks:100L,item:{id:\"minecraft:stone\",count:1}}",
        );
        let fang_vis = summon_target_at(
            game,
            "minecraft:evoker_fangs",
            [8.5, 220.0, 8.5],
            "{Fire:1s,HasVisualFire:1b,Warmup:100}",
        );
        tick_fire_entities(game);
        let Target::Fangs(fid) = fang else {
            unreachable!()
        };
        let Target::Other(eid) = eye else {
            unreachable!()
        };
        let Target::Spawner(sid) = spawner else {
            unreachable!()
        };
        let Target::Fangs(fvid) = fang_vis else {
            unreachable!()
        };
        assert!(
            game.fangs
                .list
                .iter()
                .find(|f| f.id == fid)
                .unwrap()
                .synced_fire
        );
        assert!(
            game.server
                .others
                .0
                .iter()
                .find(|o| o.id == eid)
                .unwrap()
                .eye
                .unwrap()
                .synced_fire
        );
        assert!(
            game.ominous_spawners
                .iter()
                .find(|s| s.id == sid)
                .unwrap()
                .synced_fire
        );
        assert!(
            game.fangs
                .list
                .iter()
                .find(|f| f.id == fvid)
                .unwrap()
                .synced_fire,
            "HasVisualFire keeps synced_fire true when Fire reaches 0"
        );
        tick_fire_entities(game);
        assert!(
            !game
                .fangs
                .list
                .iter()
                .find(|f| f.id == fid)
                .unwrap()
                .synced_fire
        );
        assert!(
            !game
                .server
                .others
                .0
                .iter()
                .find(|o| o.id == eid)
                .unwrap()
                .eye
                .unwrap()
                .synced_fire
        );
        assert!(
            !game
                .ominous_spawners
                .iter()
                .find(|s| s.id == sid)
                .unwrap()
                .synced_fire
        );
        assert!(
            game.fangs
                .list
                .iter()
                .find(|f| f.id == fvid)
                .unwrap()
                .synced_fire
        );
        for t in [fang, eye, spawner, fang_vis] {
            game.kill_target(t);
        }
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
