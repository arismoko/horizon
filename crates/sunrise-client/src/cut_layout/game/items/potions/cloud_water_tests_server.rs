#[cfg(feature = "client")]
use crate::game::client_side::potions::cloud_water_tests::offset_coord;
#[cfg(feature = "client")]
use crate::game::client_side::potions::cloud_water_tests::parse_double_bits;
#[cfg(feature = "client")]
use crate::game::client_side::potions::cloud_water_tests::parse_motion_bits;
#[cfg(feature = "client")]
use crate::game::client_side::potions::cloud_water_tests::read_witness;
#[cfg(feature = "client")]
use serde_json::Value;
#[cfg(feature = "client")]
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_core::position::ChunkPos;
#[cfg(feature = "client")]
use sunrise_gameplay::game::commands::data::DataRef;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::GameGameExt1;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::dragon_common::Breath;
#[cfg(feature = "client")]
use sunrise_sim::motion::PlayerMotion;
#[cfg(feature = "client")]
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

#[cfg(feature = "client")]
pub(crate) fn run_a_cloud_in_water_resets_its_fall_distance_and_a_dry_one_keeps_it() {
    let witness = read_witness();
    let witness: Value = serde_json::from_str(&witness).unwrap();
    assert_eq!(witness["minecraft"].as_str(), Some("26.3"));
    let blocks = witness["blocks"].as_array().unwrap();
    let cases = witness["cases"].as_array().unwrap();
    assert!(!blocks.is_empty());
    assert!(!cases.is_empty());

    with_game(|game| {
        // Load chunks -2..=8 in X and Z so the radius-32 box and its neighbor check are loaded.
        for cx in [0, 3, 6] {
            for cz in [0, 3, 6] {
                prepare_entity_ticking_neighborhood(game, ChunkPos::new(cx, cz));
            }
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add -16 -16 127 127"));
        let origin = [0_i32, 64, 0];
        for entry in blocks {
            let entry = entry.as_array().unwrap();
            let bx = origin[0] + entry[0].as_i64().unwrap() as i32;
            let by = origin[1] + entry[1].as_i64().unwrap() as i32;
            let bz = origin[2] + entry[2].as_i64().unwrap() as i32;
            let block = entry[3].as_str().unwrap();
            let cmd = format!("setblock {bx} {by} {bz} {block}");
            assert!(game.run_command(&cmd), "{cmd}");
        }

        let mut spawned = Vec::with_capacity(cases.len());
        let mut expected_observations = 0usize;
        for (index, case) in cases.iter().enumerate() {
            let n = (index + 1) as i32;
            let uuid = [2359, 0, n, 1];
            let uid_str = format!("00000937-0000-0000-0000-{n:04x}00000001");
            let offset = case["offset"].as_array().unwrap();
            let x = offset_coord(origin[0], offset[0].as_str().unwrap());
            let y = offset_coord(origin[1], offset[1].as_str().unwrap());
            let z = offset_coord(origin[2], offset[2].as_str().unwrap());
            let initial = case["initial_fall_distance"].as_str().unwrap();
            let duration = case["duration"].as_i64().unwrap();
            let radius = case["radius"]
                .as_str()
                .map_or_else(String::new, |r| format!(",Radius:{r}f"));
            let extra = case["extra_nbt"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map_or_else(String::new, |s| format!(",{s}"));
            let command = format!(
                "summon minecraft:area_effect_cloud {x} {y} {z} {{UUID:[I;2359,0,{n},1],fall_distance:{initial}d,Duration:{duration}{radius}{extra}}}"
            );
            assert!(game.run_command(&command), "{command}");
            let id = game.breath_clouds.last().unwrap().id;
            let obs = case["observations"].as_array().unwrap();
            assert!(!obs.is_empty());
            expected_observations += obs.len();
            spawned.push((id, uuid, uid_str, case));
        }

        let mut checked = 0usize;
        for tick in 0..=2usize {
            if tick > 0 {
                game.tick_potion_clouds();
            }
            for (id, uuid, uid_str, case) in &spawned {
                let name = case["name"].as_str().unwrap();
                let obs = &case["observations"].as_array().unwrap()[tick];
                assert_eq!(obs["tick"].as_u64(), Some(tick as u64));
                let expected_fall = Some(parse_double_bits(obs["fall_bits"].as_str().unwrap()));
                let expected_motion = Some(parse_motion_bits(obs));

                // Saved chunk view (`potion_cloud_to_nbt`).
                let saved = {
                    let cloud = game.breath_clouds.iter().find(|c| c.id == *id).unwrap();
                    sunrise_gameplay::game::server_side::potions_server::potion_cloud_to_nbt(
                        cloud,
                        *uuid,
                        game.persist_raw(*id),
                    )
                };
                assert_eq!(
                    saved.get("fall_distance").cloned(),
                    expected_fall,
                    "{name} tick {tick} fall_distance (chunk save)"
                );
                assert_eq!(
                    saved.get("Motion").cloned(),
                    expected_motion,
                    "{name} tick {tick} Motion (chunk save)"
                );

                // Live `/data get` command and accessor view (`DataRef::Entity`).
                assert!(
                    game.run_command(&format!("data get entity {uid_str} fall_distance")),
                    "{name} tick {tick} /data get fall_distance"
                );
                assert!(
                    game.run_command(&format!("data get entity {uid_str} Motion")),
                    "{name} tick {tick} /data get Motion"
                );
                let target = game.target_with_uuid(*uuid).unwrap();
                let live = game.data_of(&DataRef::Entity(target)).unwrap();
                assert_eq!(
                    live.get("fall_distance").cloned(),
                    expected_fall,
                    "{name} tick {tick} fall_distance (/data get)"
                );
                assert_eq!(
                    live.get("Motion").cloned(),
                    expected_motion,
                    "{name} tick {tick} Motion (/data get)"
                );
                checked += 1;
            }
        }
        assert_eq!(
            checked, expected_observations,
            "every witness observation must be verified"
        );

        // Naturally spawned dragon-breath clouds (`potion: None`) share the same fluid step
        // in `tick_breath`, gated on tick admission (`ticked_entities_through`).
        let flow_case = cases
            .iter()
            .find(|c| c["name"].as_str() == Some("flowing_water_nonzero_motion"))
            .unwrap();
        let flow_t0 = &flow_case["observations"].as_array().unwrap()[0];
        let flow_t1 = &flow_case["observations"].as_array().unwrap()[1];
        let make_dragon_cloud = |id: u32| Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            at: [1.5, 64.0, 20.5],
            motion: [0.05, 0.0, -0.02],
            fall_distance: 20.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 0.4,
            age: 0,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        };
        let admitted_id = game.take_entity_id();
        let admitted_uuid = [2359, 1, 0, 1];
        game.persist.uuids.insert(admitted_id, admitted_uuid);
        game.breath_clouds.push(make_dragon_cloud(admitted_id));
        game.server.ticked_entities_through = admitted_id;

        let late_id = game.take_entity_id();
        let late_uuid = [2359, 1, 0, 2];
        game.persist.uuids.insert(late_id, late_uuid);
        game.breath_clouds.push(make_dragon_cloud(late_id));

        game.tick_breath();

        let admitted_target = game.target_with_uuid(admitted_uuid).unwrap();
        let admitted_live = game.data_of(&DataRef::Entity(admitted_target)).unwrap();
        assert_eq!(
            admitted_live.get("fall_distance").cloned(),
            Some(parse_double_bits(flow_t1["fall_bits"].as_str().unwrap())),
            "dragon breath admitted fall_distance"
        );
        assert_eq!(
            admitted_live.get("Motion").cloned(),
            Some(parse_motion_bits(flow_t1)),
            "dragon breath admitted Motion"
        );

        let late_target = game.target_with_uuid(late_uuid).unwrap();
        let late_live = game.data_of(&DataRef::Entity(late_target)).unwrap();
        assert_eq!(
            late_live.get("fall_distance").cloned(),
            Some(parse_double_bits(flow_t0["fall_bits"].as_str().unwrap())),
            "dragon breath unadmitted fall_distance must stay unchanged"
        );
        assert_eq!(
            late_live.get("Motion").cloned(),
            Some(parse_motion_bits(flow_t0)),
            "dragon breath unadmitted Motion must stay unchanged"
        );
    });
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
