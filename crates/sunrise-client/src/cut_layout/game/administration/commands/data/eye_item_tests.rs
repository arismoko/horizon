use std::path::PathBuf;
use sunrise_core::nbt::{Tag, parse_snbt};
use {
    sunrise_gameplay::game::server_side::commands::data::DataRef,
    sunrise_gameplay_common::game::commands_common::Target,
};

pub(crate) fn load_witness() -> String {
    let raw = std::env::var_os("SUNRISE_EYE_ITEM_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_EYE_ITEM_WITNESS is unset; run tools/test_eye_item_parity.py, which records the witness"
        )
    });
    let path = PathBuf::from(raw);
    let path = if path.is_dir() {
        path.join("eye_item.tsv")
    } else {
        path
    };
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read eye_item witness {} (run tools/test_eye_item_parity.py): {e}",
            path.display()
        )
    })
}

pub(crate) fn parse_exact_snbt(text: &str) -> Tag {
    let (tag, used) = parse_snbt(text).expect("valid recorded 26.3 SNBT");
    assert_eq!(
        used,
        text.len(),
        "trailing bytes in recorded 26.3 SNBT: {text}"
    );
    tag
}

pub(crate) fn saved_item(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Tag {
    let tag = game.data_of(&DataRef::Entity(Target::Other(id))).unwrap();
    tag.get("Item")
        .cloned()
        .expect("an eye of ender always saves Item")
}

/// Matches the real vanilla 26.3 witness recorded by `tools/test_eye_item_parity.py`.
#[test]
#[ignore = "run by tools/test_eye_item_parity.py"]
fn a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload() {
    run_a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload();
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::commands_server::data::eye_item_tests::run_a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload;
