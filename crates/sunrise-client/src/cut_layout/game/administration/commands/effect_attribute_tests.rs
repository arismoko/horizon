//! #1988: on a NoAI zombie, `/attribute` reads the modifier Jump Boost,
//! Absorption and Invisibility store when given (`MobEffect.addAttributeModifiers`
//! from `LivingEntity.onEffectAdded`), and none when the effect was loaded
//! from `active_effects` (`readAdditionalSaveData` adds none). The values are
//! `MobEffects`': `minecraft:effect.jump_boost` 1.0 per level and
//! `minecraft:effect.absorption` 4.0 per level, both `add_value`, and
//! `minecraft:effect.waypoint_transmit_range_hide` -1.0, `add_multiplied_total`.
use std::sync::Arc;
use sunrise_level::chunk::Chunk;

pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(game.run_command("forceload add 0 0"));
        // A zombie cannot be summoned in peaceful.
        assert!(game.run_command("difficulty normal"));
        test(game);
    });
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::commands_server::world_cmds::effect_attribute_tests::read;

#[test]
#[ignore = "daily"]
fn attribute_reads_the_modifier_a_given_effect_stored_and_none_for_a_loaded_one() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:zombie 2.5 64 2.5 {NoAI:1b,Tags:["given"],UUID:[I;1988,0,0,1]}"#
        ));
        assert!(game.run_command(
            r#"summon minecraft:zombie 4.5 64 2.5 {NoAI:1b,Tags:["loaded"],AbsorptionAmount:8f,UUID:[I;1988,0,0,2],attributes:[{id:"minecraft:waypoint_transmit_range",base:100d}],active_effects:[{id:"minecraft:jump_boost",amplifier:1b,duration:-1},{id:"minecraft:absorption",amplifier:1b,duration:-1},{id:"minecraft:invisibility",amplifier:0b,duration:-1}]}"#
        ));
        assert!(game.run_command(
            "attribute @e[tag=given,limit=1] minecraft:waypoint_transmit_range base set 100"
        ));
        for effect in [
            "jump_boost infinite 1",
            "absorption infinite 1",
            "invisibility infinite 0",
        ] {
            assert!(game.run_command(&format!(
                "effect give @e[tag=given,limit=1] minecraft:{effect}"
            )));
        }
        let rows = [
            ("safe_fall_distance", "jump_boost", "5.0", "2.0", "3.0"),
            ("max_absorption", "absorption", "8.0", "8.0", "0.0"),
            (
                "waypoint_transmit_range",
                "waypoint_transmit_range_hide",
                "0.0",
                "-1.0",
                "100.0",
            ),
        ];
        for (attribute, modifier, given, amount, loaded) in rows {
            let get =
                |tag: &str| format!("attribute @e[tag={tag},limit=1] minecraft:{attribute} get");
            let value = |tag: &str| {
                format!(
                    "attribute @e[tag={tag},limit=1] minecraft:{attribute} modifier value get minecraft:effect.{modifier}"
                )
            };
            assert_eq!(
                read(game, &get("given")).as_deref(),
                Some(given),
                "{attribute}"
            );
            assert_eq!(
                read(game, &value("given")).as_deref(),
                Some(amount),
                "{attribute}"
            );
            assert_eq!(
                read(game, &get("loaded")).as_deref(),
                Some(loaded),
                "{attribute}"
            );
            assert_eq!(read(game, &value("loaded")), None, "{attribute}");
        }
    });
}

/// `AttributeCommand.getAttributeModifier` reads `getModifier`, which finds a
/// transient modifier too: a mod's transient `test:mod` (1.5) on a zombie's
/// `safe_fall_distance` reads 1.5, as it did when the attribute lived in the
/// generic map.
#[test]
#[ignore = "daily"]
fn modifier_value_get_finds_a_mods_transient_modifier() {
    use sunrise_sim::mobs::finalize::{self, Attr, Op};
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:zombie 2.5 64 2.5 {NoAI:1b,Tags:["t"],UUID:[I;1988,0,0,5]}"#
        ));
        let m = game
            .mobs
            .iter_mut()
            .find(|m| m.kind.id == "minecraft:zombie")
            .expect("the summoned zombie");
        finalize::add_transient_modifier(m, Attr::SafeFallDistance, "test:mod", 1.5, Op::AddValue);
        let value =
            "attribute @e[tag=t,limit=1] minecraft:safe_fall_distance modifier value get test:mod";
        assert_eq!(read(game, value).as_deref(), Some("1.5"));
        assert_eq!(
            read(
                game,
                "attribute @e[tag=t,limit=1] minecraft:safe_fall_distance get"
            )
            .as_deref(),
            Some("4.5")
        );
    });
}

/// #4473: a zombie loaded with the scale modifier a content mod's effect
/// stored before the save (`testfx:effect/stun_scale`, 0.5,
/// `add_multiplied_base`): `/attribute` reads its map with what the load
/// kept, so it reports the modifier and the scale it makes, and removes it;
/// the loaded scale sizes the zombie (`onAttributeUpdated(SCALE)`).
#[test]
#[ignore = "daily"]
fn attribute_reports_and_removes_a_reloaded_mobs_saved_effect_modifier() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:zombie 2.5 64 2.5 {NoAI:1b,Tags:["r"],UUID:[I;4473,0,0,1],attributes:[{id:"minecraft:scale",base:1d,modifiers:[{id:"testfx:effect/stun_scale",amount:0.5d,operation:"add_multiplied_base"}]}]}"#
        ));
        let scale = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.mobs
                .iter()
                .find(|m| m.kind.id == "minecraft:zombie")
                .expect("the summoned zombie")
                .extra
                .model_scale
        };
        assert_eq!(scale(game), 1.5);
        let value = "attribute @e[tag=r,limit=1] minecraft:scale modifier value get testfx:effect/stun_scale";
        let get = "attribute @e[tag=r,limit=1] minecraft:scale get";
        assert_eq!(read(game, value).as_deref(), Some("0.5"));
        assert_eq!(read(game, get).as_deref(), Some("1.5"));
        assert!(game.run_command(
            "attribute @e[tag=r,limit=1] minecraft:scale modifier remove testfx:effect/stun_scale"
        ));
        assert_eq!(read(game, value), None);
        assert_eq!(read(game, get).as_deref(), Some("1.0"));
        assert_eq!(scale(game), 1.0);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
