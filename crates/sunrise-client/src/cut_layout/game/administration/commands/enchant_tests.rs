use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::finalize::{MAINHAND, item_stack};
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

pub(crate) const TRIDENT: &str = "minecraft:trident";
pub(crate) const RIPTIDE: &str = "minecraft:riptide";

pub(crate) fn riptide(stack: Option<&ItemStack>) -> u32 {
    let stack = stack.expect("held stack");
    assert_eq!(&*stack.item, TRIDENT);
    stack.enchantment_level(RIPTIDE)
}

#[test]
#[ignore = "daily"]
fn enchant_reports_single_target_errors_and_counts_only_compatible_living_targets() {
    if run_fresh_cases(
        "cut_layout::game::server_side::commands::enchant_tests::enchant_reports_single_target_errors_and_counts_only_compatible_living_targets",
        "SUNRISE_TEST_ENCHANT_LIVING_TARGETS",
        &["living"],
    ) {
        return;
    }
    with_game(|game| {
        game.player = None;
        game.build_registries();
        game.set_allow_commands(true);
        for command in [
            "summon zombie 4 64 4 {NoAI:1b,equipment:{mainhand:{id:\"minecraft:trident\",count:1}}}",
            "summon armor_stand 5 64 4 {ShowArms:1b,equipment:{mainhand:{id:\"minecraft:trident\",count:1}}}",
            "summon marker 6 64 4",
            "summon armor_stand 7 64 4",
            "summon zombie 8 64 4 {NoAI:1b,equipment:{mainhand:{id:\"minecraft:stick\",count:1,components:{\"minecraft:custom_name\":\"Not a trident\"}}}}",
            "summon zombie 9 64 4 {NoAI:1b,Tags:[\"empty_hand\"]}",
        ] {
            assert!(game.as_console(|g| g.run_command(command)), "{command}");
        }
        let empty_hand = ItemStack::new(TRIDENT, 0);
        game.mobs
            .last_mut()
            .unwrap()
            .set_equipment_stack(MAINHAND, Some(empty_hand.clone()));
        for (target, level, key, arg) in [
            ("@n[type=marker]", 4, "level", Some("4")),
            ("@n[type=marker]", 3, "entity", None),
            ("@n[type=armor_stand,x=7,y=64,z=4]", 3, "itemless", None),
            ("@n[tag=empty_hand]", 3, "itemless", None),
            (
                "@n[type=zombie,x=8,y=64,z=4]",
                3,
                "incompatible",
                Some("Not a trident"),
            ),
        ] {
            enchant_outcome(game, target, level, false, 0, &format!("failed.{key}"), arg);
        }
        enchant_outcome(game, "@e", 3, true, 2, "success.multiple", None);
        assert_eq!(riptide(game.mobs[0].equipment_stacks()[0].as_ref()), 3);
        assert_eq!(riptide(game.armor_stands[0].equipment.get(0)), 3);
        enchant_outcome(game, "@e", 3, false, 0, "failed", None);
        assert_eq!(
            item_stack(game.mobs.last().unwrap(), MAINHAND),
            Some(empty_hand)
        );
    });
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::enchant_tests::enchant_outcome;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
