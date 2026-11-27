use crate::game::server_side::commands::data::eye_item_tests::load_witness;
use crate::game::server_side::commands::data::eye_item_tests::parse_exact_snbt;
use crate::game::server_side::commands::data::eye_item_tests::saved_item;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;

pub(crate) fn run_a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload() {
    let witness = load_witness();
    assert!(
        witness.contains("# minecraft_version\t26.3"),
        "witness must be tagged with 26.3"
    );
    with_game(|game| {
        let mut checked = 0usize;
        for line in witness.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let case = cols.next().expect("case column");
            let nbt = cols.next().expect("summon_nbt column");
            let live_expected = parse_exact_snbt(cols.next().expect("live_item_snbt column"));
            let reloaded_expected =
                parse_exact_snbt(cols.next().expect("reloaded_item_snbt column"));
            assert_eq!(cols.next(), None, "unexpected extra column in {case}");

            game.summon_with_nbt("minecraft:eye_of_ender", [5.5, 64.0, 5.5], nbt)
                .unwrap();
            let eye = game.server.others.0.last().unwrap().id;
            assert_eq!(
                saved_item(game, eye),
                live_expected,
                "26.3 live, {case}: {nbt}"
            );

            let other = game.server.others.0.last().unwrap().clone();
            let uuid = game.uuid_for(eye);
            let saved = game.other_to_nbt(&other, uuid);
            game.server.others.0.retain(|o| o.id != eye);
            let reloaded = game
                .load_other_transferred(saved, "minecraft:eye_of_ender", None)
                .unwrap();
            assert_eq!(
                saved_item(game, reloaded),
                reloaded_expected,
                "26.3 reloaded, {case}: {nbt}"
            );
            game.server.others.0.retain(|o| o.id != reloaded);
            checked += 1;
        }
        assert!(checked > 0, "expected at least one 26.3 witness row");
    });
}
