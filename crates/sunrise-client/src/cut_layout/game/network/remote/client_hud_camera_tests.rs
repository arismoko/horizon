// 26.3 witness: real Hud, recorded into `.local` by `tools/test_camera_parity.py`.
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use std::collections::BTreeMap;
use sunrise_protocol::{
    entity_data::{Value, serializer},
    packets::play::{self, Clientbound as C},
};

pub(in crate::game) const SIZE: [u32; 2] = [427, 240];
pub(in crate::game) const LOCAL: i32 = 7;
pub(in crate::game) const WATCHED: i32 = 50;
pub(in crate::game) const MOB: i32 = 31;

pub(in crate::game) type Sprite = (String, i32, i32, i32, i32);
pub(in crate::game) type Groups = BTreeMap<&'static str, Vec<Sprite>>;

pub(in crate::game) const GROUPS: [(&str, &[&str]); 7] = [
    (
        "hotbar",
        &[
            "hud/hotbar",
            "hud/hotbar_selection",
            "hud/hotbar_offhand_left",
            "hud/hotbar_offhand_right",
            "hud/hotbar_attack_indicator_background",
        ],
    ),
    (
        "armor",
        &["hud/armor_full", "hud/armor_half", "hud/armor_empty"],
    ),
    (
        "hearts",
        &[
            "hud/heart/container",
            "hud/heart/full",
            "hud/heart/half",
            "hud/heart/absorbing_full",
            "hud/heart/absorbing_half",
        ],
    ),
    (
        "food",
        &["hud/food_empty", "hud/food_half", "hud/food_full"],
    ),
    ("air", &["hud/air", "hud/air_empty"]),
    (
        "vehicle",
        &[
            "hud/heart/vehicle_container",
            "hud/heart/vehicle_full",
            "hud/heart/vehicle_half",
        ],
    ),
    ("xp", &["hud/experience_bar_background"]),
];

pub(in crate::game) fn group_of(name: &str) -> Option<&'static str> {
    GROUPS
        .iter()
        .find(|(_, names)| names.contains(&name))
        .map(|(group, _)| *group)
}

/// The recorded vanilla draws of one case, whole sprites only, grouped by layer in draw order.
pub(in crate::game) fn vanilla_groups(case: &serde_json::Value) -> Groups {
    let mut groups = Groups::new();
    for d in case["draws"].as_array().unwrap() {
        if d["type"] == "sprite"
            && d.get("region").is_none()
            && let Some(name) = d["sprite"].as_str()
            && let Some(group) = group_of(name)
        {
            let n = |k: &str| d[k].as_i64().unwrap() as i32;
            groups.entry(group).or_default().push((
                name.to_string(),
                n("x"),
                n("y"),
                n("w"),
                n("h"),
            ));
        }
    }
    groups
}

/// The same sprites, decoded from the quads a frame of the game drew.
pub(in crate::game) fn drawn_groups(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> Groups {
    let batch = game.hud_layers(SIZE);
    let rects: Vec<(&str, [f32; 4])> = GROUPS
        .iter()
        .flat_map(|(_, names)| names.iter())
        .map(|name| {
            let r = *game
                .client
                .gui
                .atlas
                .gui_rect(name)
                .unwrap_or_else(|| panic!("the atlas holds {name}"));
            (*name, [r.u0, r.v0, r.u1, r.v1])
        })
        .collect();
    let mut groups = Groups::new();
    for q in batch.vertices.as_chunks::<6>().0 {
        let Some((name, _)) = rects
            .iter()
            .find(|(_, uv)| q[0].uv == [uv[0], uv[1]] && q[2].uv == [uv[2], uv[3]])
        else {
            continue;
        };
        groups.entry(group_of(name).unwrap()).or_default().push((
            name.to_string(),
            q[0].pos[0] as i32,
            q[0].pos[1] as i32,
            (q[2].pos[0] - q[0].pos[0]) as i32,
            (q[2].pos[1] - q[0].pos[1]) as i32,
        ));
    }
    groups
}

pub(in crate::game) fn id(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    registry: &str,
    name: &str,
) -> i32 {
    game.client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get(registry)
        .unwrap()
        .id(name)
        .unwrap()
}

pub(in crate::game) fn spawn(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    entity: i32,
    kind: &str,
) {
    let kind = id(game, "minecraft:entity_type", kind);
    game.handle_packet(C::AddEntity(play::AddEntity {
        id: entity,
        uuid: entity as u128 + 1000,
        kind,
        position: [10.0, 64.0, 12.0],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

pub(in crate::game) fn data(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    entity: i32,
    values: Vec<(u8, i32, Value)>,
) {
    game.handle_packet(C::SetEntityData(play::SetEntityData { id: entity, values }))
        .unwrap();
}

pub(in crate::game) fn attributes(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    entity: i32,
    values: &[(&str, f64)],
) {
    let values = values
        .iter()
        .map(|(name, base)| play::AttributeSnapshot {
            attribute: game.motion.attributes.id(name).unwrap().registry_id(),
            base: *base,
            modifiers: vec![],
        })
        .collect();
    game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
        id: entity,
        values,
    }))
    .unwrap();
}

pub(in crate::game) fn num(v: &serde_json::Value, key: &str) -> f64 {
    v[key].as_f64().unwrap()
}

/// A mount at `entity` carrying `rider`, with the health the case gives it.
pub(in crate::game) fn mount(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    entity: i32,
    rider: i32,
    max: f64,
    health: f64,
) {
    spawn(game, entity, "minecraft:horse");
    attributes(game, entity, &[("minecraft:max_health", max)]);
    data(
        game,
        entity,
        vec![(9, serializer::FLOAT, Value::Float(health as f32))],
    );
    game.handle_packet(C::SetPassengers(play::SetPassengers {
        vehicle: entity,
        passengers: vec![rider],
    }))
    .unwrap();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::client_hud_camera_tests::set_up;

#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn hud_bars_follow_the_camera_player_like_26_3() {
    run_hud_bars_follow_the_camera_player_like_26_3();
}

pub(in crate::game) fn run_hud_bars_follow_the_camera_player_like_26_3() {
    let path = std::env::var("SUNRISE_HUD_CAMERA_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_HUD_CAMERA_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    let mut cases = 0;
    for line in witness.lines() {
        cases += 1;
        let recorded: serde_json::Value = serde_json::from_str(line).unwrap();
        let case = &recorded["case"];
        let expected = vanilla_groups(&recorded);
        let label = format!(
            "case {cases} {} {}",
            case["gameMode"].as_str().unwrap(),
            case["cameraKind"].as_str().unwrap()
        );
        let has_xp = |g: &Groups| !g.get("xp").unwrap_or(&vec![]).is_empty();
        let survival = matches!(case["gameMode"].as_str(), Some("survival" | "adventure"));
        assert_eq!(
            has_xp(&expected),
            survival,
            "{label}: the recording draws the experience bar"
        );
        with_client_game(|game| {
            set_up(game, case);
            let drawn = drawn_groups(game);
            assert_eq!(
                has_xp(&drawn),
                has_xp(&expected),
                "{label}: the HUD drew the experience bar"
            );
            assert_eq!(drawn, expected, "{label}: sprites drawn per layer");
        });
    }
    assert_eq!(cases, 11, "every recorded case ran");
}
