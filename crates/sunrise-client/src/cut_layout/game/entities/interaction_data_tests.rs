//! `/data` on a live `minecraft:interaction`: `EntityDataAccessor` loads the
//! edited compound into the same entity (`Interaction.readAdditionalSaveData`:
//! width, height, response and the attack/interaction records), keeping its
//! UUID (tools/test_interaction_data_parity.py measures the same on vanilla
//! 26.3).
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::{Tag, parse_snbt};

const EDITED: &str = "00000042-0000-0000-0000-000000000001";

fn game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        test(game);
    });
}

/// Runs `command`: whether it succeeded and what it said first.
fn run(game: &mut sunrise_gameplay::game::ServerGame<'_>, command: &str) -> (bool, Option<String>) {
    game.server.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.server.chat_capture.take().unwrap_or_default();
    (ok, said.first().map(sunrise_game_data::text::Text::plain))
}

#[test]
#[ignore = "daily"]
fn data_edits_reach_the_same_live_interaction() {
    game(|game| {
        assert!(game.run_command("summon interaction 4.5 100 4.5 {UUID:[I;66,0,0,1]}"));
        assert_eq!(game.server.others.0.len(), 1);
        let id = game.server.others.0[0].id;
        let modified = Some("Modified entity data of Interaction".to_owned());
        assert_eq!(
            run(
                game,
                &format!("data merge entity {EDITED} {{width:2.5f,height:1.25f,response:1b}}")
            ),
            (true, modified.clone())
        );
        let tag = game
            .command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Other(id))
            .unwrap();
        assert_eq!(tag.get("width"), Some(&Tag::Float(2.5f32.to_bits())));
        assert_eq!(tag.get("height"), Some(&Tag::Float(1.25f32.to_bits())));
        assert_eq!(tag.get("response"), Some(&Tag::Byte(1)));
        // A record, then its removal; a removed width reads its default again.
        assert_eq!(
            run(
                game,
                &format!(
                    "data merge entity {EDITED} {{attack:{{player:[I;1,2,3,4],timestamp:5L}}}}"
                )
            ),
            (true, modified.clone())
        );
        let tag = game
            .command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Other(id))
            .unwrap();
        assert_eq!(
            tag.get("attack"),
            Some(&parse_snbt("{player:[I;1,2,3,4],timestamp:5L}").unwrap().0)
        );
        assert!(run(game, &format!("data remove entity {EDITED} width")).0);
        let tag = game
            .command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Other(id))
            .unwrap();
        assert_eq!(tag.get("width"), Some(&Tag::Float(1.0f32.to_bits())));
        // The same entity: its id, UUID and place.
        assert_eq!(
            (game.server.others.0.len(), game.server.others.0[0].id),
            (1, id)
        );
        assert_eq!(tag.get("UUID"), Some(&Tag::IntArray(vec![66, 0, 0, 1])));
        assert_eq!(game.server.others.0[0].position, Vec3::new(4.5, 100.0, 4.5));
    });
}

/// Added controls (after the baseline): the live settings, not the loaded
/// compound, are what the entity saves; a stale compound gives way; a
/// removed record is gone; a marker and an item display keep their own
/// state; every entity keeps its identity.
#[test]
#[ignore = "daily"]
fn the_live_settings_are_what_the_interaction_saves() {
    use {
        sunrise_gameplay_common::game::interactions_common::InteractionState,
        sunrise_gameplay_common::game::interactions_common::PlayerAction,
    };
    game(|game| {
        assert!(game.run_command("summon interaction 4.5 100 4.5 {UUID:[I;66,0,0,1]}"));
        assert!(game.run_command("summon marker 8.5 100 4.5 {UUID:[I;66,0,0,3],data:{a:1}}"));
        assert!(game.run_command(
        "summon item_display 12.5 100 4.5 {UUID:[I;66,0,0,4],transformation:[1f,0f,0f,0f,0f,1f,0f,0f,0f,0f,1f,0f,0f,0f,0f,1f]}"
    ));
        let find = |g: &sunrise_gameplay::game::ServerGame<'_>, uuid: [i32; 4]| {
            g.server
                .others
                .0
                .iter()
                .find(|o| g.persist.uuids.get(&o.id) == Some(&uuid))
                .unwrap()
                .clone()
        };
        let (interaction, marker, display) = (
            find(game, [66, 0, 0, 1]),
            find(game, [66, 0, 0, 3]),
            find(game, [66, 0, 0, 4]),
        );
        let default = InteractionState {
            width: 1.0,
            height: 1.0,
            response: false,
            attack: None,
            interaction: None,
        };
        assert_eq!(
            interaction.interaction,
            Some(default),
            "summoned with its defaults"
        );
        assert_eq!((marker.interaction, display.interaction), (None, None));
        // The edit reaches the live settings.
        assert!(game.run_command(&format!(
        "data merge entity {EDITED} {{width:2.5f,response:1b,attack:{{player:[I;1,2,3,4],timestamp:5L}}}}"
    )));
        let live = find(game, [66, 0, 0, 1]);
        let edited = InteractionState {
            width: 2.5,
            response: true,
            attack: Some(PlayerAction {
                player: [1, 2, 3, 4],
                timestamp: 5,
            }),
            ..default
        };
        assert_eq!(live.interaction, Some(edited));
        assert_eq!(
            live.clone().interaction,
            Some(edited),
            "a copy carries them"
        );
        // Saved from them, with no loaded compound and over a stale one.
        let id = live.id;
        for raw in [
        None,
        Some(parse_snbt("{width:9f,height:9f,response:0b,interaction:{player:[I;9,9,9,9],timestamp:1L},keep:1b}").unwrap().0),
    ] {
        if let Some(nbt) = raw.as_ref() {
            game.persist.raw.insert(id, nbt.clone());
        } else {
            game.persist.raw.remove(&id);
        }
        let tag = game.command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Other(id)).unwrap();
        assert_eq!(tag.get("width"), Some(&Tag::Float(2.5f32.to_bits())));
        assert_eq!(tag.get("height"), Some(&Tag::Float(1.0f32.to_bits())));
        assert_eq!(tag.get("response"), Some(&Tag::Byte(1)));
        assert_eq!(
            tag.get("attack"),
            Some(&parse_snbt("{player:[I;1,2,3,4],timestamp:5L}").unwrap().0)
        );
        assert!(tag.get("interaction").is_none(), "no stale record");
        assert_eq!(tag.get("keep").is_some(), raw.is_some(), "other keys stay");
    }
        // A removed record and a removed width.
        assert!(game.run_command(&format!("data remove entity {EDITED} attack")));
        assert!(game.run_command(&format!("data remove entity {EDITED} width")));
        let state = find(game, [66, 0, 0, 1]).interaction.unwrap();
        assert_eq!((state.attack, state.width), (None, 1.0));
        let tag = game
            .command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Other(id))
            .unwrap();
        assert!(tag.get("attack").is_none());
        // The others keep their own state, and every entity its identity.
        assert!(
            game.run_command("data merge entity 00000042-0000-0000-0000-000000000003 {data:{b:2}}")
        );
        let (m, d) = (find(game, [66, 0, 0, 3]), find(game, [66, 0, 0, 4]));
        assert_eq!((m.id, m.interaction), (marker.id, None));
        assert_eq!(
            (d.id, d.interaction, format!("{:?}", d.display)),
            (display.id, None, format!("{:?}", display.display))
        );
        assert_eq!(game.server.others.0.len(), 3);
        assert_eq!(find(game, [66, 0, 0, 1]).id, interaction.id);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
