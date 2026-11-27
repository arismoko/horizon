use serde::Deserialize;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use {
    sunrise_gameplay::game::server_side::commands::data::DataRef,
    sunrise_gameplay_common::game::commands_common::Target,
};

#[derive(Deserialize)]
struct Witness {
    minecraft: String,
    cases: Vec<SummonCase>,
    merges: Vec<MergeCase>,
}

#[derive(Deserialize)]
struct SummonCase {
    label: String,
    kind: String,
    summon_nbt: String,
    after_summon_bits: [u32; 2],
    after_tick_bits: [u32; 2],
    after_reload_bits: [u32; 2],
}

#[derive(Deserialize)]
struct MergeCase {
    label: String,
    kind: String,
    initial_nbt: String,
    merge_nbt: String,
    after_merge_bits: [u32; 2],
}

fn rotation(tag: &Tag) -> Option<&Tag> {
    tag.get("Rotation")
}

fn rotation_tag(bits: [u32; 2]) -> Tag {
    Tag::List(vec![Tag::Float(bits[0]), Tag::Float(bits[1])])
}

/// Vanilla 26.3 witness: `area_effect_cloud` and `falling_block` keep their loaded `Rotation` (#2680).
/// Recorded into `.local` by `tools/test_cloud_falling_loaded_rotation_parity.py`.
#[test]
#[ignore = "run by tools/test_cloud_falling_loaded_rotation_parity.py"]
fn a_cloud_and_a_falling_block_save_the_rotation_they_were_loaded_with() {
    run_a_cloud_and_a_falling_block_save_the_rotation_they_were_loaded_with();
}

pub(crate) fn run_a_cloud_and_a_falling_block_save_the_rotation_they_were_loaded_with() {
    let witness_path = std::env::var_os("SUNRISE_CLOUD_FALLING_ROTATION_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_CLOUD_FALLING_ROTATION_WITNESS is unset; run tools/test_cloud_falling_loaded_rotation_parity.py"
        )
    });
    let witness_path = std::path::PathBuf::from(witness_path);
    let witness_path = if witness_path.is_dir() {
        witness_path.join("cloud_falling_loaded_rotation.json")
    } else {
        witness_path
    };
    let raw = std::fs::read_to_string(&witness_path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (record via tools/test_cloud_falling_loaded_rotation_parity.py): {e}",
            witness_path.display()
        )
    });
    let witness: Witness = serde_json::from_str(&raw).expect("valid 26.3 rotation witness JSON");
    assert_eq!(witness.minecraft, "26.3");
    assert!(!witness.cases.is_empty());
    assert!(!witness.merges.is_empty());

    with_game(|game| {
        let mut cloud_index = 0usize;
        let mut falling_index = 0usize;
        let mut default_cloud_bits = None;
        for case in &witness.cases {
            let label = &case.label;
            assert_eq!(
                case.after_summon_bits, case.after_tick_bits,
                "{label}: vanilla 26.3 tick kept Rotation"
            );
            assert_eq!(
                case.after_summon_bits, case.after_reload_bits,
                "{label}: vanilla 26.3 reload kept Rotation"
            );
            let expected_live = rotation_tag(case.after_summon_bits);
            let expected_saved = rotation_tag(case.after_reload_bits);
            match case.kind.as_str() {
                "minecraft:area_effect_cloud" => {
                    if case.label == "cloud_default" {
                        default_cloud_bits = Some(case.after_reload_bits);
                    }
                    let x = 2.5 + (cloud_index as f64) * 2.0;
                    cloud_index += 1;
                    game.summon_with_nbt(&case.kind, [x, 64.0, 4.5], &case.summon_nbt)
                        .unwrap();
                    let id = game.breath_clouds.last().unwrap().id;
                    let live = game.data_of(&DataRef::Entity(Target::Cloud(id))).unwrap();
                    assert_eq!(
                        rotation(&live),
                        Some(&expected_live),
                        "{label}: cloud /data get"
                    );
                    let cloud = game.breath_clouds.last().unwrap();
                    let saved =
                        sunrise_gameplay::game::server_side::potions_server::potion_cloud_to_nbt(
                            cloud,
                            [1, 2, 3, 4],
                            game.persist_raw(id),
                        );
                    assert_eq!(
                        rotation(&saved),
                        Some(&expected_saved),
                        "{label}: cloud chunk save"
                    );
                }
                "minecraft:falling_block" => {
                    let x = 2.5 + (falling_index as f64) * 2.0;
                    falling_index += 1;
                    let falling = game
                        .summon_with_nbt(&case.kind, [x, 64.0, 8.5], &case.summon_nbt)
                        .unwrap()
                        .unwrap();
                    let live = game.data_of(&DataRef::Entity(falling)).unwrap();
                    assert_eq!(
                        rotation(&live),
                        Some(&expected_live),
                        "{label}: falling block /data get"
                    );
                }
                other => panic!("unexpected witness entity kind: {other}"),
            }
        }
        // A cloud with no loaded raw compound matches the default 26.3 witness.
        let cloud = game.breath_clouds.last().unwrap();
        let saved = sunrise_gameplay::game::server_side::potions_server::potion_cloud_to_nbt(
            cloud,
            [1, 2, 3, 4],
            None,
        );
        let default_bits = default_cloud_bits.expect("cloud_default witness case");
        assert_eq!(
            rotation(&saved),
            Some(&rotation_tag(default_bits)),
            "cloud without loaded raw compound"
        );

        let first = &witness.merges[0];
        let falling = game
            .summon_with_nbt(&first.kind, [2.5, 64.0, 12.5], &first.initial_nbt)
            .unwrap()
            .unwrap();
        for merge in &witness.merges {
            let label = &merge.label;
            let patch = sunrise_command::snbt::parse_fully(&merge.merge_nbt).unwrap();
            let old = game.data_of(&DataRef::Entity(falling)).unwrap();
            let merged = sunrise_sim::data_cmd::merged(&old, &patch);
            game.set_data_of(&DataRef::Entity(falling), merged).unwrap();
            let live = game.data_of(&DataRef::Entity(falling)).unwrap();
            let expected = rotation_tag(merge.after_merge_bits);
            assert_eq!(
                rotation(&live),
                Some(&expected),
                "{label}: falling block /data merge"
            );
        }
    });
}
