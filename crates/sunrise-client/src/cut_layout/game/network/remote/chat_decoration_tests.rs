/// `ChatType.Bound.decorate` and `decorateNarration`: a private message
/// shows as "Steve whispers to you: hi" and is narrated "Steve says hi";
/// a chat type the registry lacks is `minecraft:chat`'s.
#[test]
fn chat_is_narrated_with_its_chat_types_narration_decoration() {
    use serde_json::json;
    use sunrise_core::nbt::{JavaString, Tag, parse_snbt};
    use sunrise_gameplay_common::game::remote::chat::chat_type_decorate;
    use {
        sunrise_gameplay_common::game::remote::chat::Decoration,
        sunrise_gameplay_common::game::remote::play,
    };
    let bound = play::ChatTypeBound {
        chat_type: 0,
        name: Tag::String(JavaString::from("Steve")),
        target: None,
    };
    let content = json!("hi");
    let (incoming, _) = parse_snbt(
        r#"{chat:{translation_key:"commands.message.display.incoming",parameters:["sender","content"],style:{color:"gray",italic:1b}},narration:{translation_key:"chat.type.text.narrate",parameters:["sender","content"]}}"#,
    )
    .unwrap();
    let chat = chat_type_decorate(Some(&incoming), Decoration::Chat, &bound, &content);
    assert_eq!(chat["translate"], "commands.message.display.incoming");
    assert_eq!(chat["color"], "gray");
    assert_eq!(chat["with"], json!(["Steve", "hi"]));
    assert_eq!(
        chat_type_decorate(Some(&incoming), Decoration::Narration, &bound, &content),
        json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
    );
    assert_eq!(
        chat_type_decorate(None, Decoration::Narration, &bound, &content),
        json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
    );
    assert_eq!(
        chat_type_decorate(None, Decoration::Chat, &bound, &content)["translate"],
        "chat.type.text"
    );
}

/// `ChatType.bind` looks the key up in the world's registry: a pack's `minecraft:say_command`
/// replaces the entry, so the decoration a client applies to `/say` is the pack's.
#[test]
fn a_packs_say_command_chat_type_is_the_decoration_clients_apply() {
    use serde_json::json;
    use sunrise_core::nbt::{JavaString, Tag};
    use sunrise_gameplay_common::game::remote::chat::{Decoration, chat_type_decorate};
    use sunrise_gameplay_common::game::remote::play;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), b"{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let files = [(
        "chat_type/minecraft:say_command".to_owned(),
        vec![json!({
            "chat": {
                "translation_key": "pack.announcement",
                "parameters": ["sender", "content"],
                "style": {"color": "red"}
            },
            "narration": {"translation_key": "chat.type.text.narrate", "parameters": ["sender", "content"]}
        })
        .to_string()
        .into_bytes()],
    )];
    let (packed, admitted) = sunrise_sim::lan::data::SyncData::stage_registry_definitions(
        root.path(),
        &defaults,
        &sunrise_world::loot::PackOverlay::new(&files),
        &Default::default(),
        &Default::default(),
        |_| true,
    );
    admitted.unwrap();
    let mut wire = Vec::new();
    packed.append_encoded("minecraft:chat_type", &mut wire);
    let ((namespace, path), say_command, from_pack) = &wire[0];
    assert_eq!(
        (namespace.as_str(), path.as_str(), *from_pack),
        ("minecraft", "say_command", true)
    );
    let bound = play::ChatTypeBound {
        chat_type: 0,
        name: Tag::String(JavaString::from("Steve")),
        target: None,
    };
    let decorated = chat_type_decorate(Some(say_command), Decoration::Chat, &bound, &json!("hi"));
    assert_eq!(
        decorated,
        json!({"color": "red", "translate": "pack.announcement", "with": ["Steve", "hi"]})
    );
}

/// `/teammsg` binds `minecraft:team_msg_command_incoming` with the team's name as the target:
/// a pack's replacement of the entry is the decoration, and its `target` parameter is the team.
#[test]
fn a_packs_team_msg_command_incoming_decorates_with_the_team_as_target() {
    use serde_json::json;
    use sunrise_core::nbt::{JavaString, Tag};
    use sunrise_gameplay_common::game::remote::chat::{Decoration, chat_type_decorate};
    use sunrise_gameplay_common::game::remote::play;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), b"{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let files = [(
        "chat_type/minecraft:team_msg_command_incoming".to_owned(),
        vec![json!({
            "chat": {
                "translation_key": "pack.team",
                "parameters": ["target", "sender", "content"]
            },
            "narration": {"translation_key": "pack.team.narrate", "parameters": ["sender", "content"]}
        })
        .to_string()
        .into_bytes()],
    )];
    let (packed, admitted) = sunrise_sim::lan::data::SyncData::stage_registry_definitions(
        root.path(),
        &defaults,
        &sunrise_world::loot::PackOverlay::new(&files),
        &Default::default(),
        &Default::default(),
        |_| true,
    );
    admitted.unwrap();
    let mut wire = Vec::new();
    packed.append_encoded("minecraft:chat_type", &mut wire);
    let (_, incoming, from_pack) = &wire[0];
    assert!(*from_pack);
    let bound = play::ChatTypeBound {
        chat_type: 0,
        name: Tag::String(JavaString::from("Steve")),
        target: Some(Tag::String(JavaString::from("Red"))),
    };
    let decorated = chat_type_decorate(Some(incoming), Decoration::Chat, &bound, &json!("hi"));
    assert_eq!(
        decorated,
        json!({"translate": "pack.team", "with": ["Red", "Steve", "hi"]})
    );
}

/// The client resolves a chat type by the server's network id, in the server's sync order: a
/// pack's extra chat type before `minecraft:chat` moves every later id.
#[test]
fn a_chat_type_resolves_by_the_servers_network_id() {
    use serde_json::json;
    use sunrise_core::nbt::{JavaString, Tag, parse_snbt};
    use sunrise_gameplay_common::game::remote::chat::{Decoration, chat_type_decorate};
    use sunrise_gameplay_common::game::remote::play;
    use sunrise_protocol::packets::configuration::{RegistryData, RegistryEntry};
    let entry = |id: &str, key: &str| {
        RegistryEntry {
        id: id.to_owned(),
        data: Some(
            parse_snbt(&format!(
                r#"{{chat:{{translation_key:"{key}",parameters:["sender","content"]}},narration:{{translation_key:"{key}.narrate",parameters:["sender","content"]}}}}"#
            ))
            .unwrap()
            .0,
        ),
    }
    };
    let mut registries = sunrise_protocol::registry::Dynamic::default();
    registries.add(RegistryData {
        registry: "minecraft:chat_type".to_owned(),
        entries: vec![
            entry("test:extra", "test.extra"),
            entry("minecraft:chat", "chat.type.text"),
        ],
    });
    let bound = |chat_type| play::ChatTypeBound {
        chat_type,
        name: Tag::String(JavaString::from("Steve")),
        target: None,
    };
    for (id, key) in [(0, "test.extra"), (1, "chat.type.text")] {
        let bound = bound(id);
        let chat_type = registries.data("minecraft:chat_type", bound.chat_type);
        let decorated = chat_type_decorate(chat_type, Decoration::Chat, &bound, &json!("hi"));
        assert_eq!(decorated["translate"], key);
    }
}
