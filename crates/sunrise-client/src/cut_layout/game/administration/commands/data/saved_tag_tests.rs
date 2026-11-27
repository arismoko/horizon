use std::collections::HashSet;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::commands::data::DataRef;
use sunrise_gameplay_common::game::commands_common::Target;

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::commands_server::data::saved_tag_tests::with_command_game;

#[test]
#[ignore = "daily"]
fn tag_commands_and_explicit_empty_loads_save_without_empty_tags() {
    with_command_game("tags", tag_commands_and_explicit_empty);
}
pub(crate) fn tag_commands_and_explicit_empty(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let start = game.world.game_time;
    let uuid = "00000148-0000-0000-0000-000000000001";
    assert!(game.run_command("summon pig 4.5 200 4.5 {UUID:[I;328,0,0,1],Health:10f,NoAI:1b,NoGravity:1b,PortalCooldown:17}"));
    let target = game.target_with_uuid([328, 0, 0, 1]).unwrap();
    assert!(matches!(target, Target::Mob(_)));
    assert!(game.tags_of(target).is_empty());
    assert!(
        game.data_of(&DataRef::Entity(target))
            .unwrap()
            .get("Tags")
            .is_none()
    );
    for tagged in [true, false, true, false] {
        assert!(game.run_command(&format!(
            "tag {uuid} {} audit",
            if tagged { "add" } else { "remove" }
        )));
        let expected: Vec<String> = if tagged { vec!["audit".into()] } else { vec![] };
        assert_eq!(game.tags_of(target), expected);
        for _ in 0..2 {
            let saved = game.data_of(&DataRef::Entity(target)).unwrap();
            assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(vec![328, 0, 0, 1])));
            assert_eq!(saved.get("Health"), Some(&Tag::Float(10.0_f32.to_bits())));
            assert_eq!(saved.get("NoAI"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("NoGravity"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(17)));
            assert_eq!(
                saved.get("Tags"),
                if tagged {
                    Some(Tag::List(vec![Tag::String("audit".into())]))
                } else {
                    None
                }
                .as_ref()
            );
            assert_eq!(game.tags_of(target), expected);
        }
    }
    assert!(game.run_command(
        "summon pig 8.5 200 4.5 {UUID:[I;328,0,0,2],NoAI:1b,Tags:[],PortalCooldown:23}"
    ));
    let empty = game.target_with_uuid([328, 0, 0, 2]).unwrap();
    assert!(game.tags_of(empty).is_empty());
    let saved = game.data_of(&DataRef::Entity(empty)).unwrap();
    assert!(saved.get("Tags").is_none());
    assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(23)));
    assert!(game.run_command("tag 00000148-0000-0000-0000-000000000002 add keep"));
    assert_eq!(game.tags_of(empty), ["keep"]);
    assert_eq!(
        game.data_of(&DataRef::Entity(empty)).unwrap().get("Tags"),
        Some(&Tag::List(vec![Tag::String("keep".into())]))
    );
    assert_eq!(game.world.game_time, start);
}

/// `/data get` on a cloud that was never saved shows `Entity.saveWithoutId`'s
/// common fields, as 26.3 prints them (#13057).
#[test]
#[ignore = "daily"]
fn a_live_area_effect_cloud_reads_with_the_common_entity_fields() {
    with_command_game("cloud", |game| {
        assert!(game.run_command("summon area_effect_cloud 4.5 200 4.5 {UUID:[I;328,0,0,3]}"));
        let target = game.target_with_uuid([328, 0, 0, 3]).unwrap();
        let read = game.data_of(&DataRef::Entity(target)).unwrap();
        let zero = Tag::Double(0);
        assert_eq!(read.get("Motion"), Some(&Tag::List(vec![zero.clone(); 3])));
        assert_eq!(read.get("fall_distance"), Some(&zero));
        assert_eq!(read.get("Air"), Some(&Tag::Short(300)));
        assert_eq!(read.get("OnGround"), Some(&Tag::Byte(0)));
        assert_eq!(read.get("Invulnerable"), Some(&Tag::Byte(0)));
        assert_eq!(read.get("PortalCooldown"), Some(&Tag::Int(0)));
    });
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::commands_server::data::saved_tag_tests::say;

/// The `Tags` key of the compound the entity saves (`Entity.saveWithoutId`).
pub(crate) fn saved_tags(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    target: Target,
) -> Option<Tag> {
    game.data_of(&DataRef::Entity(target))
        .unwrap()
        .get("Tags")
        .cloned()
}

pub(crate) fn tag_list(tags: &[&str]) -> Tag {
    Tag::List(tags.iter().map(|t| Tag::String((*t).into())).collect())
}

/// `TagCommand.listTags` sends through `CommandResponseTracker.sendFeedback`'s
/// default element type, `NON_ZERO`: one entity of the selection holding tags
/// prints the single-target form, and the count in the multiple form is the
/// number of entities with tags, not the number of targets.
#[test]
#[ignore = "daily"]
fn tag_list_counts_only_the_entities_that_have_tags() {
    with_command_game("tag-list", |game| {
        for (uuid, x) in [("1", "4.5"), ("2", "8.5")] {
            assert!(game.run_command(&format!(
                "summon pig {x} 200 4.5 {{UUID:[I;329,0,0,{uuid}],NoAI:1b,Silent:1b,NoGravity:1b}}"
            )));
        }
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000001 add audit"));
        let said = say(game, "tag @e[type=minecraft:pig] list");
        assert_eq!(said.len(), 1);
        assert_eq!(
            said[0].translation_key(),
            Some("commands.tag.list.single.success")
        );
        assert_eq!(said[0].plain(), "Pig has 1 tag(s): audit");
        // Both tagged: the multiple form, counting the tagged entities.
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000002 add audit"));
        let said = say(game, "tag @e[type=minecraft:pig] list");
        assert_eq!(
            said[0].translation_key(),
            Some("commands.tag.list.multiple.success")
        );
        assert_eq!(said[0].plain(), "The 2 entities have 1 total tag(s): audit");
    });
}

/// `TagCommand.addTag`/`removeTag` store a tag on an ominous item spawner
/// (`OminousItemSpawner.readAdditionalSaveData` keeps the compound, its chunk
/// saves it), and `Entity.saveWithoutId` writes `Tags` only for a nonempty set,
/// so removing the last tag of a player or any entity leaves no key.
#[test]
#[ignore = "daily"]
fn the_last_tag_leaves_no_key_and_a_spawner_keeps_its_tags() {
    with_command_game("tag-remove", |game| {
        assert!(game.run_command("tag @s add audit"));
        assert_eq!(saved_tags(game, Target::Player), Some(tag_list(&["audit"])));
        assert!(game.run_command("tag @s remove audit"));
        assert_eq!(saved_tags(game, Target::Player), None);

        assert!(game.run_command(
            "summon ominous_item_spawner 4.5 200 4.5 {UUID:[I;329,0,0,3],NoGravity:1b}"
        ));
        let spawner = game.target_with_uuid([329, 0, 0, 3]).unwrap();
        assert!(matches!(spawner, Target::Spawner(_)));
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000003 add audit"));
        assert_eq!(game.tags_of(spawner), ["audit"]);
        assert_eq!(saved_tags(game, spawner), Some(tag_list(&["audit"])));
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000003 remove audit"));
        assert_eq!(saved_tags(game, spawner), None);
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000003 add audit"));
        // The entity the chunk saves carries it, and loading it back keeps it.
        let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), false);
        let saved = chunks
            .iter()
            .flat_map(|(_, entities)| entities)
            .find(|t| t.get("id") == Some(&Tag::String("minecraft:ominous_item_spawner".into())))
            .cloned()
            .expect("the spawner's saved entity");
        assert_eq!(saved.get("Tags"), Some(&tag_list(&["audit"])));
        let before = game.ominous_spawners.len();
        game.load_ominous_item_spawner(saved);
        assert_eq!(game.ominous_spawners.len(), before + 1);
        let reloaded = game.ominous_spawners.last().unwrap().id;
        assert_eq!(game.tags_of(Target::Spawner(reloaded)), ["audit"]);
    });
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
