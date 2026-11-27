//! `player_give_stack` against a real LAN guest and `item_from_snbt` on the public host.

#![cfg(test)]

use mod_api::{Host, ItemJsonError, ItemStack, PlayerDataRefusal, prelude::*};
use sunrise_core::nbt::parse_snbt;
use sunrise_gameplay_common::game::tables::GameItems;
use sunrise_items::item_nbt;
use {
    sunrise_gameplay::game::native_entity_refs::tests::{run_fresh_cases, with_game},
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::api_values::TryIntoApi,
};

fn api_item(game: &sunrise_gameplay::game::ServerGame<'_>, snbt: &str) -> Option<ItemStack> {
    let host: &dyn Host = game;
    host.item_from_snbt(snbt)
}

/// What a mod reads from SNBT and writes back is the compound `ItemStack.CODEC` started
/// from, through the API's own stack and back.
#[test]
#[ignore = "daily"]
fn an_snbt_item_goes_through_the_api_unchanged() {
    if run_fresh_cases(
        "cut_layout::game::native_give_stack_tests::an_snbt_item_goes_through_the_api_unchanged",
        "SUNRISE_NATIVE_GIVE_STACK_CHILD",
        &["roundtrip"],
    ) {
        return;
    }
    with_guests(|game| {
        for snbt in [
            r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:enchantments":{"minecraft:sharpness":3},"minecraft:custom_name":"Blade","minecraft:lore":["one","two"],"minecraft:custom_data":{husksync:{v:1b}}}}"#,
            r#"{id:"minecraft:stone",count:64}"#,
            r#"{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}}"#,
            r#"{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"Log"},author:"guest13",pages:[{raw:"one"}]}}}"#,
        ] {
            let engine = item_nbt::item_from_snbt(snbt).unwrap_or_else(|error| panic!("{error}"));
            let api = api_item(game, snbt).unwrap();
            assert_eq!(
                Some(&api),
                engine.clone().try_into_api().ok().as_ref(),
                "{snbt}"
            );
            let (tag, _) = parse_snbt(snbt).unwrap();
            assert_eq!(item_nbt::item_to_nbt(&engine, game.items()), tag, "{snbt}");
        }
    });
}

/// This API refuses with None when the grammar or saved-item codec refuses the input.
#[test]
#[ignore = "daily"]
fn item_from_snbt_refuses_what_is_not_one_item_compound() {
    if run_fresh_cases(
        "cut_layout::game::native_give_stack_tests::item_from_snbt_refuses_what_is_not_one_item_compound",
        "SUNRISE_NATIVE_GIVE_STACK_CHILD",
        &["snbt"],
    ) {
        return;
    }
    with_guests(|game| {
        let host: &dyn Host = game;
        let count = |snbt: &str| host.item_from_snbt(snbt).map(|s| s.count());
        assert_eq!(
            count(r#"{id:"stone"}"#),
            Some(1),
            "stone is minecraft:stone"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",Count:3}"#), Some(1));
        assert_eq!(
            count(
                r#"{id:"minecraft:bundle",count:1,components:{"minecraft:bundle_contents":[{id:"minecraft:stone",count:1}]}}"#
            ),
            Some(1),
            "a registered nested item is accepted"
        );
        assert_eq!(
            count(
                r#"{id:"minecraft:bundle",count:1,components:{"minecraft:bundle_contents":[{id:"nomod:missing",count:1}]}}"#
            ),
            None,
            "an unknown nested item is refused at the same boundary"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",count:3,Count:9}"#), Some(3));
        assert_eq!(count(r#"{id:"minecraft:stone",count:99}"#), Some(99));
        assert_eq!(count(r#"{id:"minecraft:stone",count:1}"#), Some(1));
        assert_eq!(
            count(r#"{id:"minecraft:stone",count:4294967297L}"#),
            Some(1),
            "Codec.INT narrows via Number.intValue before intRange(1, 99)"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",count:4294967296L}"#), None);
        assert_eq!(
            count(r#"{id:"minecraft:stone",count:100}"#),
            None,
            "a count past 99 is refused, not clamped"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",count:0}"#), None);
        assert_eq!(count(r#"{id:"minecraft:stone",count:-1}"#), None);
        assert_eq!(count(r#"{id:"minecraft:stone",count:"5"}"#), None);
        assert_eq!(count("  {id:\"minecraft:stone\"} \n\u{3000}"), Some(1));
        assert_eq!(count("stone"), None, "no compound");
        assert_eq!(count("-"), None, "no item");
        assert_eq!(count(r#"[{id:"minecraft:stone"}]"#), None);
        assert_eq!(
            count(r#"{id:"minecraft:stone"} x"#),
            None,
            "text after the compound"
        );
        assert_eq!(
            count(r#"{id:"minecraft:stone"}{id:"minecraft:stone"}"#),
            None
        );
        assert_eq!(count(r#"{id:"minecraft:air",count:1}"#), None);
        assert_eq!(count(r#"{id:"nomod:thing",count:1}"#), None);
        assert_eq!(count(r#"{count:1}"#), None);
        assert_eq!(count(r#"{id:"stone",components:1}"#), None);
        assert_eq!(
            count(r#"{id:"stone",components:{"minecraft:creative_slot_lock":{}}}"#),
            None
        );
        assert_eq!(
            count(r#"{id:"minecraft:diamond_sword",components:{"minecraft:damage":-1}}"#),
            None
        );
        assert_eq!(
            count(r#"{id:"minecraft:stone",components:{"nomod:unknown":1}}"#),
            None
        );
        assert_eq!(
            host.item_from_snbt(r#"{id:"diamond_sword",components:{"damage":5}}"#)
                .and_then(|s| s.damage_component()),
            Some(5)
        );
        let nested = |levels: usize| {
            format!(
                r#"{{id:"minecraft:stone",components:{{"minecraft:custom_data":{{a:{}0{}}}}}}}"#,
                "[".repeat(levels),
                "]".repeat(levels)
            )
        };
        assert_eq!(count(&nested(8)), Some(1));
        assert_eq!(count(&nested(600)), None, "nested past 512 levels");
    });
}

#[test]
#[ignore = "daily"]
fn item_from_snbt_keeps_custom_data_larger_than_the_binary_quota() {
    if run_fresh_cases(
        "cut_layout::game::native_give_stack_tests::item_from_snbt_keeps_custom_data_larger_than_the_binary_quota",
        "SUNRISE_NATIVE_GIVE_STACK_CHILD",
        &["long-text"],
    ) {
        return;
    }
    with_game(|game| {
        let payload = "x".repeat(2 * 1024 * 1024 + 1);
        let snbt = format!(
            r#"{{id:"minecraft:stone",count:3,components:{{"minecraft:custom_data":{{data:"{payload}"}}}}}}"#
        );
        let stack = api_item(game, &snbt).expect("in-memory text may exceed the binary quota");
        assert_eq!(stack.count(), 3);
        let data = stack
            .components()
            .and_then(|patch| patch.as_nbt().get(&"minecraft:custom_data".into()))
            .and_then(|tag| tag.get("data"));
        let Some(mod_api::Tag::String(data)) = data else {
            panic!("the item must retain its custom data");
        };
        assert_eq!(data.0.len(), payload.len());
        assert!(data.0.iter().copied().eq(payload.encode_utf16()));
    });
}

/// A given stack goes into the inventory where it fits, components and all; the rest is
/// thrown for the player alone, with no pickup delay.
#[test]
#[ignore = "daily"]
fn a_given_stack_fills_the_inventory_then_drops_for_its_player() {
    if run_fresh_cases(
        "cut_layout::game::native_give_stack_tests::a_given_stack_fills_the_inventory_then_drops_for_its_player",
        "SUNRISE_NATIVE_GIVE_STACK_CHILD",
        &["give"],
    ) {
        return;
    }
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let rock = r#"{id:"minecraft:stone",count:10,components:{"minecraft:custom_name":"Rock"}}"#;
        let named = api_item(game, rock).unwrap();
        let engine = item_nbt::item_from_snbt(rock).unwrap();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| {
                let dirt = Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 64));
                g.inventory.iter_mut().for_each(|s| *s = dirt.clone());
                g.inventory[5] = Some(sunrise_items::items::ItemStack {
                    count: 60,
                    ..engine.clone()
                });
            })
            .unwrap();
        let before = game.dropped.len();
        assert_eq!(b.give_stack(game, named.clone()), Ok(None));
        let g = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(g.inventory[5].as_ref().map(|s| s.count), Some(64));
        assert_eq!(game.dropped.len(), before + 1);
        let dropped = game.dropped.last().unwrap();
        assert_eq!(dropped.stack.count, 6);
        assert_eq!(dropped.stack.extra, engine.extra);
        assert_eq!(dropped.pickup_delay, 0, "no pickup delay");
        assert_eq!(
            dropped.target,
            Some(sunrise_sim::lan::playerdata::uuid_ints(13)),
            "only its player picks it up"
        );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| g.inventory[7] = None)
            .unwrap();
        assert_eq!(b.give_stack(game, named), Ok(None));
        let g = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(g.inventory[7].as_ref(), Some(&engine));
        assert_eq!(game.dropped.len(), before + 2);
        let fake = game.dropped.last().unwrap();
        assert_eq!(
            fake.stack,
            sunrise_items::items::ItemStack { count: 1, ..engine }
        );
        assert_eq!((fake.pickup_delay, fake.age), (32767, 5999));
    });
}

/// This API refuses a dead player with its own name and gives nothing.
#[test]
#[ignore = "daily"]
fn a_dead_player_is_given_nothing() {
    if run_fresh_cases(
        "cut_layout::game::native_give_stack_tests::a_dead_player_is_given_nothing",
        "SUNRISE_NATIVE_GIVE_STACK_CHILD",
        &["dead"],
    ) {
        return;
    }
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let rock = api_item(game, r#"{id:"minecraft:stone",count:3}"#).unwrap();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        let before = game.dropped.len();
        assert_eq!(b.give_stack(game, rock), Err(PlayerDataRefusal::Dead));
        assert_eq!(
            b.with_inventory(game, |view| view.iter().next().is_none()),
            Some(true)
        );
        assert_eq!(game.dropped.len(), before);
    });
}

/// `Host::item_to_json` / `Host::item_from_json` match `ItemStack.OPTIONAL_CODEC` over `JsonOps`:
/// empty stack (`{}`), plain stack, and a stack with text, enchantments, damage, float, boolean
/// and explicit component removal.
#[test]
#[ignore = "daily"]
fn item_json_roundtrips_plain_component_and_empty_stacks() {
    if run_fresh_cases(
        "cut_layout::game::native_give_stack_tests::item_json_roundtrips_plain_component_and_empty_stacks",
        "SUNRISE_NATIVE_GIVE_STACK_CHILD",
        &["json-roundtrip"],
    ) {
        return;
    }
    with_game(|game| {
        let host: &dyn Host = game;

        // Empty stack <-> {}.
        assert_eq!(host.item_to_json(None).unwrap(), "{}");
        assert_eq!(host.item_from_json("{}").unwrap(), None);

        // Plain stack <-> {"id", "count"}.
        let plain_expected = serde_json::json!({"id": "minecraft:stone", "count": 64});
        let plain = host
            .item_from_json(r#"{"id":"minecraft:stone","count":64}"#)
            .unwrap()
            .expect("plain stack");
        assert_eq!(plain.count(), 64);
        let plain_encoded = host.item_to_json(Some(&plain)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&plain_encoded).unwrap(),
            plain_expected
        );
        assert_eq!(host.item_from_json(&plain_encoded).unwrap(), Some(plain));

        // Stack with several component kinds: styled text component (real JSON bool),
        // enchantments, integer damage, float attack charge, boolean glint override, and
        // removal of a prototype component (`!minecraft:rarity`).
        let rich_expected = serde_json::json!({
            "id": "minecraft:diamond_sword",
            "count": 1,
            "components": {
                "minecraft:custom_name": {"text": "Blade", "color": "gold", "bold": true},
                "minecraft:enchantments": {"minecraft:sharpness": 3},
                "minecraft:damage": 5,
                "minecraft:minimum_attack_charge": 0.5,
                "minecraft:enchantment_glint_override": true,
                "!minecraft:rarity": {}
            }
        });
        let rich = host
            .item_from_json(&rich_expected.to_string())
            .unwrap()
            .expect("multi-component stack");
        assert_eq!(rich.damage_component(), Some(5));
        let rich_encoded = host.item_to_json(Some(&rich)).unwrap();
        let rich_parsed: serde_json::Value = serde_json::from_str(&rich_encoded).unwrap();
        assert_eq!(rich_parsed, rich_expected);
        assert_eq!(
            rich_parsed["components"]["minecraft:custom_name"]["bold"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(host.item_from_json(&rich_encoded).unwrap(), Some(rich));

        // Explicit removal of `minecraft:damage` on a damageable item writes `"!minecraft:damage": {}`.
        let removed_expected = serde_json::json!({
            "id": "minecraft:diamond_sword",
            "count": 1,
            "components": {
                "!minecraft:damage": {}
            }
        });
        let removed = host
            .item_from_json(&removed_expected.to_string())
            .unwrap()
            .expect("damage-removed sword");
        let removed_encoded = host.item_to_json(Some(&removed)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&removed_encoded).unwrap(),
            removed_expected
        );
        assert_eq!(
            host.item_from_json(&removed_encoded).unwrap(),
            Some(removed)
        );
    });
}

/// Decode refusals report the failing field name through `ItemJsonError`.
#[test]
#[ignore = "daily"]
fn item_json_decode_errors_name_the_failing_field() {
    if run_fresh_cases(
        "cut_layout::game::native_give_stack_tests::item_json_decode_errors_name_the_failing_field",
        "SUNRISE_NATIVE_GIVE_STACK_CHILD",
        &["json-errors"],
    ) {
        return;
    }
    with_game(|game| {
        let host: &dyn Host = game;

        for (input, expected_field) in [
            (r#"{"count":1}"#, "id"),
            (r#"{"id":"nomod:missing","count":1}"#, "id"),
            (r#"{"id":"minecraft:air","count":1}"#, "id"),
            (r#"{"id":"minecraft:stone","count":100}"#, "count"),
            (r#"{"id":"minecraft:stone","count":true}"#, "count"),
            (r#"{"id":"minecraft:stone","components":1}"#, "components"),
            (
                r#"{"id":"minecraft:diamond_sword","components":{"minecraft:damage":-1}}"#,
                "minecraft:damage",
            ),
            (
                r#"{"id":"minecraft:diamond_sword","components":{"minecraft:damage":true}}"#,
                "minecraft:damage",
            ),
            (
                r#"{"id":"minecraft:stone","components":{"minecraft:custom_name":{"text":"Hi","bold":1}}}"#,
                "minecraft:custom_name",
            ),
            (
                r#"{"id":"minecraft:diamond_sword","components":{"!minecraft:damage":1}}"#,
                "!minecraft:damage",
            ),
            (
                r#"{"id":"minecraft:stone","components":{"nomod:unknown":1}}"#,
                "nomod:unknown",
            ),
            (
                r#"{"id":"minecraft:stone","components":{"minecraft:creative_slot_lock":{}}}"#,
                "minecraft:creative_slot_lock",
            ),
        ] {
            let error = host.item_from_json(input).unwrap_err();
            assert_eq!(error.field(), Some(expected_field), "{input}: {error}");
            assert!(
                error.to_string().contains(expected_field),
                "{input}: {error}"
            );
        }

        assert!(matches!(
            host.item_from_json("{bad"),
            Err(ItemJsonError::InvalidJson { .. })
        ));
    });
}
