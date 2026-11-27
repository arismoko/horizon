use std::collections::HashMap;
use std::collections::HashSet;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::nbt::parse_snbt;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay::game::server_side::persist::ChunkKey;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::commands_common::Target;
#[cfg(test)]
use sunrise_sim::entity_nbt;
use sunrise_sim::save::uuid_text;
use {
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

struct WitnessCase {
    kind: String,
    uuid: String,
    uuid_u128: u128,
    summon_cmd: String,
    edit_cmds: Vec<String>,
    summon_pos: Vec3,
    moved_pos: Vec3,
    moved_pos_tag: Tag,
    moved_item: Tag,
    reloaded_item: Tag,
}

fn parse_exact_snbt(text: &str) -> Tag {
    let (tag, used) = parse_snbt(text).unwrap_or_else(|e| panic!("invalid snbt {text:?}: {e}"));
    assert_eq!(used, text.len(), "trailing snbt in {text:?}");
    tag
}

fn entity_data_snbt(text: &str) -> Tag {
    let (_, body) = text
        .split_once(" has the following entity data: ")
        .unwrap_or_else(|| panic!("unexpected /data get output: {text:?}"));
    parse_exact_snbt(body)
}

fn vec3_from_pos_tag(tag: &Tag) -> Vec3 {
    let Tag::List(items) = tag else {
        panic!("expected Pos list, got {tag:?}");
    };
    let [Tag::Double(x), Tag::Double(y), Tag::Double(z)] = items.as_slice() else {
        panic!("expected 3 doubles in Pos, got {tag:?}");
    };
    Vec3::new(f64::from_bits(*x), f64::from_bits(*y), f64::from_bits(*z))
}

fn load_witness_26_3() -> (i32, i32, Vec<WitnessCase>) {
    let path = std::env::var("SUNRISE_THROWN_ITEM_SAVE_WITNESS")
        .expect("run through tools/test_thrown_item_save_parity.py, which records the witness");
    let witness_text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read thrown-item save witness {path:?} (run through tools/test_thrown_item_save_parity.py): {e}"
        )
    });
    let witness: serde_json::Value = serde_json::from_str(&witness_text).unwrap();
    assert_eq!(witness["minecraft"].as_str(), Some("26.3"));
    let records = witness["records"].as_array().unwrap();
    let mut summoned: Vec<(String, String, u128, String, Vec3)> = Vec::new();
    let mut seen_uuids: HashSet<String> = HashSet::new();
    let mut edit_cmds: HashMap<String, Vec<String>> = HashMap::new();
    let mut moved_pos: HashMap<String, (Vec3, Tag)> = HashMap::new();
    let mut moved_item: HashMap<String, Tag> = HashMap::new();
    let mut reloaded_item: HashMap<String, Tag> = HashMap::new();
    let mut unload_xz: Option<(i32, i32)> = None;
    for rec in records {
        let cmd = rec["cmd"].as_str().unwrap();
        let text = rec["text"].as_str().unwrap();
        if let Some(rest) = cmd.strip_prefix("/summon ") {
            assert_eq!(rec["results"], serde_json::json!([[true, 1]]), "{cmd}");
            let mut parts = rest.splitn(5, ' ');
            let kind = parts.next().unwrap().to_owned();
            let x: f64 = parts.next().unwrap().parse().unwrap();
            let y: f64 = parts.next().unwrap().parse().unwrap();
            let z: f64 = parts.next().unwrap().parse().unwrap();
            let nbt = parts.next().unwrap().to_owned();
            let tag = parse_exact_snbt(&nbt);
            let ints = entity_nbt::uuid(&tag).unwrap();
            let uuid = uuid_text(ints);
            let uuid_u128 = sunrise_sim::lan::playerdata::ints_uuid(ints);
            assert!(
                seen_uuids.insert(uuid.clone()),
                "duplicate summon UUID in witness: {uuid}"
            );
            summoned.push((kind, uuid, uuid_u128, cmd.to_owned(), Vec3::new(x, y, z)));
        } else if let Some(rest) = cmd
            .strip_prefix("/data modify entity ")
            .or_else(|| cmd.strip_prefix("/data merge entity "))
        {
            assert_eq!(rec["results"], serde_json::json!([[true, 1]]), "{cmd}");
            let (uuid, _) = rest.split_once(' ').unwrap();
            edit_cmds
                .entry(uuid.to_owned())
                .or_default()
                .push(cmd.to_owned());
        } else if let Some(rest) = cmd.strip_prefix("/execute unless loaded ") {
            assert_eq!(rec["results"], serde_json::json!([[true, 1]]), "{cmd}");
            let mut coords = rest.split_whitespace();
            let x: i32 = coords.next().unwrap().parse().unwrap();
            let _y: i32 = coords.next().unwrap().parse().unwrap();
            let z: i32 = coords.next().unwrap().parse().unwrap();
            assert!(
                unload_xz.replace((x, z)).is_none(),
                "duplicate unload check"
            );
        } else if let Some(rest) = cmd.strip_prefix("/data get entity ") {
            let (uuid, field) = rest.split_once(' ').unwrap();
            let value = entity_data_snbt(text);
            match (unload_xz.is_some(), field) {
                (false, "Pos") => {
                    let pos = vec3_from_pos_tag(&value);
                    assert!(
                        moved_pos.insert(uuid.to_owned(), (pos, value)).is_none(),
                        "duplicate pre-unload Pos for {uuid}"
                    );
                }
                (false, "Item") => {
                    assert!(
                        moved_item.insert(uuid.to_owned(), value).is_none(),
                        "duplicate pre-unload Item for {uuid}"
                    );
                }
                (true, "Item") => {
                    assert!(
                        reloaded_item.insert(uuid.to_owned(), value).is_none(),
                        "duplicate post-reload Item for {uuid}"
                    );
                }
                _ => panic!("unexpected /data get record: {cmd:?}"),
            }
        }
    }
    let (x0, z0) = unload_xz.expect("witness must verify chunk unload");
    assert!(!summoned.is_empty(), "witness must contain summon cases");
    let cases: Vec<WitnessCase> = summoned
        .into_iter()
        .map(|(kind, uuid, uuid_u128, summon_cmd, summon_pos)| {
            let (moved_pos, moved_pos_tag) = moved_pos
                .remove(&uuid)
                .unwrap_or_else(|| panic!("missing pre-unload Pos for {uuid}"));
            let moved_item = moved_item
                .remove(&uuid)
                .unwrap_or_else(|| panic!("missing pre-unload Item for {uuid}"));
            let reloaded_item = reloaded_item
                .remove(&uuid)
                .unwrap_or_else(|| panic!("missing post-reload Item for {uuid}"));
            let edit_cmds = edit_cmds.remove(&uuid).unwrap_or_default();
            WitnessCase {
                kind,
                uuid,
                uuid_u128,
                summon_cmd,
                edit_cmds,
                summon_pos,
                moved_pos,
                moved_pos_tag,
                moved_item,
                reloaded_item,
            }
        })
        .collect();
    assert!(moved_pos.is_empty(), "extra pre-unload Pos observations");
    assert!(moved_item.is_empty(), "extra pre-unload Item observations");
    assert!(
        reloaded_item.is_empty(),
        "extra post-reload Item observations"
    );
    assert!(edit_cmds.is_empty(), "extra /data edit commands");
    (x0, z0, cases)
}

fn collect_projectile_uuids(
    game: &sunrise_gameplay::game::ServerGame<'_>,
) -> HashMap<String, (usize, u32)> {
    let mut by_uuid = HashMap::new();
    for (idx, p) in game.creatures.projectiles.iter().enumerate() {
        let ints = game
            .persist
            .uuid_of(p.id)
            .unwrap_or_else(|| panic!("projectile {} missing persisted UUID", p.id));
        let uuid = uuid_text(ints);
        assert!(
            by_uuid.insert(uuid.clone(), (idx, p.id)).is_none(),
            "duplicate resident projectile UUID: {uuid}"
        );
    }
    by_uuid
}

/// #2691 (26.3): thrown-item projectiles match the vanilla 26.3 tick, unload, and reload witness
/// recorded by `tools/test_thrown_item_save_parity.py`.
#[test]
#[ignore = "run by tools/test_thrown_item_save_parity.py"]
fn a_moved_thrown_item_projectile_saves_its_loaded_item() {
    run_a_moved_thrown_item_projectile_saves_its_loaded_item();
}

pub(crate) fn run_a_moved_thrown_item_projectile_saves_its_loaded_item() {
    let (x0, z0, cases) = load_witness_26_3();
    let key = (x0 >> 4, z0 >> 4);
    let keys: HashSet<ChunkKey> = [key].into();
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(key.0, key.1));
        assert!(game.run_command(&format!("forceload add {x0} {z0}")));
        game.update_chunk_tickets();
        assert!(
            game.entity_ticking_area()(x0 + 2, z0 + 2),
            "target chunk must be entity-ticking before projectile tick"
        );

        // Summon every case and apply any pre-movement /data edits while stationary.
        for case in &cases {
            let summon = case.summon_cmd.strip_prefix('/').unwrap();
            assert!(game.run_command(summon), "summon failed: {summon}");
            for edit in &case.edit_cmds {
                let edit_cmd = edit.strip_prefix('/').unwrap();
                assert!(game.run_command(edit_cmd), "edit failed: {edit_cmd}");
            }
        }

        // Run one real projectile tick so moving projectiles actually advance.
        game.server.ticked_entities_through = game.server.next_entity;
        game.tick_projectiles().unwrap();

        let live_by_uuid = collect_projectile_uuids(game);
        assert_eq!(
            live_by_uuid.len(),
            cases.len(),
            "missing or extra resident projectiles before unload"
        );
        let mut confirmed_moved = 0usize;
        for case in &cases {
            let &(_, id) = live_by_uuid
                .get(&case.uuid)
                .unwrap_or_else(|| panic!("missing live projectile {} {}", case.kind, case.uuid));
            let live = game.command_target_nbt(Target::Projectile(id)).unwrap();
            assert_eq!(
                live.get("Pos"),
                Some(&case.moved_pos_tag),
                "{} {} post-tick Pos",
                case.kind,
                case.uuid
            );
            let live_pos = vec3_from_pos_tag(live.get("Pos").unwrap());
            if case.moved_pos != case.summon_pos {
                assert_ne!(
                    live_pos, case.summon_pos,
                    "{} {} must move during tick",
                    case.kind, case.uuid
                );
                confirmed_moved += 1;
            } else {
                assert_eq!(
                    live_pos, case.summon_pos,
                    "{} {} stationary control must stay put",
                    case.kind, case.uuid
                );
            }
            assert_eq!(
                live.get("Item"),
                Some(&case.moved_item),
                "{} {} live /data get Item before unload",
                case.kind,
                case.uuid
            );
        }
        assert!(confirmed_moved > 0, "at least one projectile must move");

        let chunk = game.world.chunks.remove(&key).unwrap();
        let (records, _) = game.unloading_entity_snapshots(&keys);
        game.server.save.0.save_entities_later(records).unwrap();
        game.remove_unloading_entities(&keys);
        game.server.save.0.flush().unwrap();
        assert!(
            game.creatures.projectiles.is_empty(),
            "chunk unload failed to remove resident projectiles"
        );
        for case in &cases {
            assert!(
                game.target_by_uuid(case.uuid_u128).is_none(),
                "{} {} remained resident after chunk unload",
                case.kind,
                case.uuid
            );
        }

        // Reload the chunk from disk and compare the full typed Item by UUID.
        let records = game.server.save.0.load_entities(key.0, key.1).unwrap();
        assert_eq!(
            records.len(),
            cases.len(),
            "saved chunk has missing, duplicate, or extra entity records"
        );
        game.world.chunks.insert(key, chunk);
        game.world.queue_entity_nbt(key, records);
        game.spawn_loaded_entities();

        let reloaded_by_uuid = collect_projectile_uuids(game);
        assert_eq!(
            reloaded_by_uuid.len(),
            cases.len(),
            "missing or extra projectiles after chunk reload"
        );
        for case in &cases {
            let &(idx, id) = reloaded_by_uuid.get(&case.uuid).unwrap_or_else(|| {
                panic!("missing reloaded projectile {} {}", case.kind, case.uuid)
            });
            let reloaded = game.command_target_nbt(Target::Projectile(id)).unwrap();
            assert_eq!(
                reloaded.get("Pos"),
                Some(&case.moved_pos_tag),
                "{} {} reloaded Pos",
                case.kind,
                case.uuid
            );
            assert_eq!(
                reloaded.get("Item"),
                Some(&case.reloaded_item),
                "{} {} reloaded /data get Item",
                case.kind,
                case.uuid
            );
            let (_, resaved) = game.save_projectile(idx, &keys).unwrap();
            assert_eq!(
                resaved.get("Item"),
                Some(&case.reloaded_item),
                "{} {} reloaded chunk save Item",
                case.kind,
                case.uuid
            );
        }
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
