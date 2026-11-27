//! The host's persistent text forms, read as `ComponentSerialization.CODEC` takes them.
use mod_api::text::TextFormError;
use sunrise_command::Registries;
use sunrise_core::json_nbt::{JsonNbtError, tag_to_json};
use sunrise_core::nbt::{self, Tag};
use sunrise_game_data::text::Text;
use sunrise_sim::text_grammar::build;

impl crate::game::CommonGame<'_> {
    /// A component read as `/tellraw` takes it; None when it isn't one (an object with no
    /// content key, an empty list, a content key of the wrong kind).
    pub fn text_from_json(&self, json: &str) -> Read {
        Read(component_from_json(json, &self.ui.registries))
    }
}

/// A component, none, or a refusal, crossing to the API as one `Result`.
pub struct Read(Result<Option<Text>, TextFormError>);

impl crate::game::api_values::TryIntoApi for Read {
    type Api = Option<mod_api::Text>;
    type Error = TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.0.as_ref().map_err(Clone::clone)?.try_into_api()
    }
}

/// One JSON or SNBT component (`ComponentSerialization.CODEC`'s string, list or object
/// forms), with nothing but whitespace after it.
fn component_from_json(json: &str, registries: &Registries) -> Result<Option<Text>, TextFormError> {
    // JSON first: it decodes escapes and has a real `null`. SNBT is the fallback (and takes
    // the nesting beyond serde_json's own limit to the parser's 512).
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
        return Ok(build(&value, registries, false).map(Text::from));
    }
    let (tag, used) = match nbt::parse_snbt(json) {
        Ok(parsed) => parsed,
        // NBT's own limit of 512: the named refusal, not "not a component".
        Err(error) if error.starts_with("arguments.nbtpath.too_deep at ") => {
            return Err(TextFormError::TooDeep);
        }
        Err(_) => return Ok(None),
    };
    let read = if json.as_bytes()[used..].iter().all(u8::is_ascii_whitespace) {
        component_from_snbt(&tag, registries)
    } else {
        Ok(None)
    };
    nbt::discard_tag(tag);
    read
}

fn component_from_snbt(tag: &Tag, registries: &Registries) -> Result<Option<Text>, TextFormError> {
    let value = match tag_to_json(tag) {
        Ok(value) => value,
        Err(JsonNbtError::TooDeep) => return Err(TextFormError::TooDeep),
        Err(_) => return Ok(None),
    };
    // Only the rebuilt value reaches the capture, so a failure anywhere leaves no partial text.
    let text = build(&value, registries, true).map(Text::from);
    sunrise_game_data::component::discard_component(value);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::{Registries, Text, TextFormError};
    use crate::game::api_values::TryIntoApi;
    use mod_api::text::{TextClick, TextContent, TextNbtSource};

    fn component_from_json(json: &str) -> Option<Text> {
        super::component_from_json(json, &sunrise_command::Registries::default()).unwrap()
    }

    fn api(text: &Text) -> mod_api::Text {
        text.try_into_api().unwrap()
    }

    /// JSON and SNBT components keep their structure: styles, children, translations
    /// and click events; what is not one component is refused.
    #[test]
    fn text_from_json_reads_components_as_tellraw_does() {
        let text = api(&component_from_json(
            r#"{"text":"Hi ","color":"gold","extra":[{"text":"there","bold":true}]}"#,
        )
        .unwrap());
        assert_eq!(text.content, TextContent::Literal("Hi ".into()));
        assert_eq!(text.style.color.as_deref(), Some("gold"));
        assert_eq!(text.children.len(), 1);
        assert_eq!(text.children[0].style.bold, Some(true));
        let text = api(
            &component_from_json(r#"{"translate":"chat.type.text","with":["a","b"]}"#).unwrap(),
        );
        assert!(
            matches!(&text.content, TextContent::Translatable { key, arguments, .. } if key == "chat.type.text" && arguments.len() == 2)
        );
        let text = api(&component_from_json(
            "{text:'run',click_event:{action:run_command,command:'/spawn'}}",
        )
        .unwrap());
        assert_eq!(
            text.style.click,
            Some(TextClick::RunCommand("/spawn".into()))
        );
        assert_eq!(
            api(&component_from_json("  \"plain\"\n").unwrap()).content,
            TextContent::Literal("plain".into())
        );
        for refused in [
            "",
            "{\"text\":\"a\"} trailing",
            "{\"text\":\"open",
            "12",
            "true",
            "[]",
        ] {
            assert!(component_from_json(refused).is_none(), "{refused:?}");
        }
    }

    /// Objects and lists that are not one component are refused whole, however deep.
    const NOT_COMPONENTS: [&str; 46] = [
        "{}",
        r#"{"color":"red"}"#,
        r#"{"unrelated":"x","extra":[]}"#,
        r#"{"bold":true,"italic":false}"#,
        r#"{"text":{"a":1}}"#,
        r#"{"text":["a"]}"#,
        r#"{"translate":5}"#,
        r#"{"translate":"k","with":"a"}"#,
        r#"{"keybind":true}"#,
        r#"{"selector":["@a"]}"#,
        r#"{"score":{"name":"a"}}"#,
        r#"{"score":"a"}"#,
        r#"{"nbt":"a.b"}"#,
        r#"{"nbt":"a.b","source":"entity","block":"0 0 0"}"#,
        r#"{"type":"bogus","text":"a"}"#,
        r#"{"type":"keybind","text":"a"}"#,
        r#"{"text":"a","extra":"b"}"#,
        r#"{"text":"a","extra":[{}]}"#,
        r#"{"text":"a","extra":[{"text":"b","extra":[{"color":"red"}]}]}"#,
        r#"[{}]"#,
        r#"["a",{"color":"red"}]"#,
        r#"["a",[]]"#,
        r#"[1]"#,
        r#"{"text":"a","extra":[{"text":"b","extra":5}]}"#,
        // A number is not a string: `text` is not coerced.
        r#"{"text":1}"#,
        // `extra` is a non-empty list.
        r#"{"text":"a","extra":[]}"#,
        // A style field that is there and wrong refuses the whole component.
        r##"{"text":"a","color":"#GGGGGG"}"##,
        r#"{"text":"a","color":5}"#,
        r#"{"text":"a","bold":"yes"}"#,
        r#"{"text":"a","shadow_color":"red"}"#,
        r#"{"text":"a","insertion":1}"#,
        r#"{"text":"a","font":5}"#,
        r#"{"text":"a","click_event":{"action":"bogus"}}"#,
        r#"{"text":"a","click_event":"run_command"}"#,
        r#"{"text":"a","hover_event":{"action":"show_text"}}"#,
        r#"{"text":"a","hover_event":{"action":"show_text","value":{}}}"#,
        r#"{"text":"a","hover_event":{"action":"show_item","id":"minecraft:bad item"}}"#,
        r#"{"text":"a","hover_event":{"action":"show_item","id":"air"}}"#,
        r#"{"text":"a","hover_event":{"action":"show_item","id":"minecraft:stone","components":{"!minecraft:lore":[]}}}"#,
        // The partial result is never used: a good entry before a bad one.
        r#"["a",{"text":"b","extra":["ok",{"bad":1}]}]"#,
        r#"{"text":"a","bold":true,"extra":["ok",{"text":"b","italic":"no"}]}"#,
        // A selector and an NBT path must parse; a selector's separator stays strict.
        r#"{"selector":"@x"}"#,
        r#"{"selector":"@a","separator":5}"#,
        r#"{"nbt":"[","entity":"@s"}"#,
        r#"{"nbt":"a","entity":"@x"}"#,
        r#"{"nbt":"a","entity":"@s","interpret":true,"plain":true}"#,
    ];

    const NUMERIC_FLAGS: [&str; 10] = [
        r#"{"text":"a","bold":0}"#,
        r#"{"text":"a","bold":1}"#,
        r#"{"text":"a","italic":0}"#,
        r#"{"text":"a","italic":1}"#,
        r#"{"text":"a","underlined":0}"#,
        r#"{"text":"a","underlined":1}"#,
        r#"{"text":"a","strikethrough":0}"#,
        r#"{"text":"a","strikethrough":1}"#,
        r#"{"text":"a","obfuscated":0}"#,
        r#"{"text":"a","obfuscated":1}"#,
    ];

    #[test]
    fn text_from_json_distinguishes_json_flags_from_snbt_bytes() {
        for input in NUMERIC_FLAGS {
            assert!(component_from_json(input).is_none(), "{input}");
        }
        for flag in [
            "bold",
            "italic",
            "underlined",
            "strikethrough",
            "obfuscated",
        ] {
            for value in [false, true] {
                let json = format!(r#"{{"text":"a","{flag}":{value}}}"#);
                assert!(component_from_json(&json).is_some(), "{json}");
                let snbt = format!("{{text:a,{flag}:{}b}}", u8::from(value));
                let built = api(&component_from_json(&snbt).unwrap());
                let actual = match flag {
                    "bold" => built.style.bold,
                    "italic" => built.style.italic,
                    "underlined" => built.style.underlined,
                    "strikethrough" => built.style.strikethrough,
                    _ => built.style.obfuscated,
                };
                assert_eq!(actual, Some(value), "{snbt}");
            }
        }
    }

    #[test]
    fn text_from_json_rebuilds_hover_entity_name() {
        let input = r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","keybind":"key.jump","text":"wrong"}}}"#;
        let text = api(&component_from_json(input).unwrap());
        assert!(
            matches!(&text.style.hover, Some(mod_api::text::TextHover::Entity { name: Some(name), .. }) if name.content == TextContent::Keybind("key.jump".into()))
        );
        assert!(component_from_json(r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","text":"wrong"}}}"#).is_none());
    }

    #[test]
    fn text_from_json_refuses_objects_that_are_not_components() {
        for refused in NOT_COMPONENTS {
            assert!(component_from_json(refused).is_none(), "{refused}");
        }
        // The same objects in SNBT.
        assert!(component_from_json("{color:red}").is_none());
        assert!(component_from_json("{}").is_none());
    }

    /// An explicit `type` is built as that type, whatever other content keys sit beside it.
    #[test]
    fn text_from_json_builds_an_explicit_type_alone() {
        let content = |json: &str| api(&component_from_json(json).unwrap()).content.clone();
        assert_eq!(
            content(r#"{"type":"keybind","keybind":"key.jump","text":"wrong"}"#),
            TextContent::Keybind("key.jump".into())
        );
        assert_eq!(
            content(r#"{"type":"text","text":"right","keybind":"key.jump"}"#),
            TextContent::Literal("right".into())
        );
        let text = api(&component_from_json(
            r#"{"text":"a","extra":[{"type":"keybind","keybind":"key.use","text":"x"}]}"#,
        )
        .unwrap());
        assert_eq!(
            text.children[0].content,
            TextContent::Keybind("key.use".into())
        );
        assert!(matches!(
            content(
                r#"{"type":"nbt","source":"entity","nbt":"Health","block":"0 64 0","entity":"@s"}"#
            ),
            TextContent::Nbt { path, source: TextNbtSource::Entity(selector), .. }
                if path == "Health" && selector == "@s"
        ));
        assert!(matches!(
            content(r#"{"type":"nbt","source":"block","nbt":"Items","block":"0 64 0","entity":"@s"}"#),
            TextContent::Nbt { source: TextNbtSource::Block(pos), .. } if pos == "0 64 0"
        ));
    }

    /// Without a `type` the first kind the keys satisfy decides, in the codec's order
    /// (`text`, `translatable`, `keybind`, `score`, `selector`, `nbt`, `object`); a lenient
    /// field of the wrong kind is dropped from the built value.
    #[test]
    fn text_from_json_builds_what_the_codec_decodes() {
        let content = |json: &str| api(&component_from_json(json).unwrap()).content.clone();
        assert_eq!(
            content(r#"{"text":"a","keybind":"key.jump"}"#),
            TextContent::Literal("a".into())
        );
        assert_eq!(
            content(r#"{"selector":"@a","keybind":"key.jump"}"#),
            TextContent::Keybind("key.jump".into())
        );
        assert!(matches!(
            content(r#"{"selector":"@a","score":{"name":"x","objective":"o"}}"#),
            TextContent::Score { .. }
        ));
        assert!(matches!(
            content(r#"{"translate":"k","fallback":5}"#),
            TextContent::Translatable { key, fallback: None, .. } if key == "k"
        ));
        assert!(matches!(
            content(r#"{"sprite":"test:icon","player":{"name":"Steve"}}"#),
            TextContent::Object(mod_api::text::TextObject::Atlas { sprite, .. }) if sprite == "test:icon"
        ));
        assert!(matches!(
            content(r#"{"atlas":7,"sprite":"test:icon","player":{"name":"Steve"}}"#),
            TextContent::Object(mod_api::text::TextObject::Player { .. })
        ));
        // `{"text":1}` fails the text kind, so the next kind that decodes is used.
        assert_eq!(
            content(r#"{"text":1,"keybind":"key.use"}"#),
            TextContent::Keybind("key.use".into())
        );
    }

    /// JSON escapes decode to the character they name.
    #[test]
    fn text_from_json_decodes_json_escapes() {
        for (json, expected) in [
            (r#"{"text":"\u0041"}"#, "A"),
            // SNBT takes the same escapes.
            (r#"'\u0041\ud83d\ude00'"#, "A\u{1F600}"),
            (r#"{"text":"\""}"#, "\""),
            (r#"{"text":"\\"}"#, "\\"),
            (r#"{"text":"\/"}"#, "/"),
            (r#"{"text":"a\nb\tc\r\b\f"}"#, "a\nb\tc\r\u{8}\u{c}"),
            (r#""\u00e9\ud83d\ude00""#, "\u{e9}\u{1F600}"),
        ] {
            assert_eq!(
                api(&component_from_json(json).unwrap()).content,
                TextContent::Literal(expected.into()),
                "{json}"
            );
        }
    }

    /// A component 513 `extra` levels deep is refused with the named nesting error, not read
    /// as "not a component".
    #[test]
    fn text_from_json_names_the_nesting_refusal() {
        let deep = |levels: usize| {
            let mut json = String::from(r#"{"text":"leaf"}"#);
            for _ in 0..levels {
                json = format!(r#"{{"text":"n","extra":[{json}]}}"#);
            }
            json
        };
        assert!(matches!(
            super::component_from_json(&deep(513), &Registries::default()),
            Err(TextFormError::TooDeep)
        ));
        assert!(matches!(
            super::component_from_json(&deep(3), &Registries::default()),
            Ok(Some(_))
        ));
    }

    /// Each content kind of `ComponentSerialization`, with `extra` and arrays, is a component.
    #[test]
    fn text_from_json_accepts_each_content_kind() {
        for accepted in [
            r#"{"text":"a"}"#,
            r#"{"text":"","extra":null}"#,
            r#"{"type":null,"text":"a","bold":null,"unknown":[1]}"#,
            r#"{"text":"a","shadow_color":[1,0,0.5,1],"font":"alt","insertion":"i"}"#,
            r##"{"text":"a","color":"#FF00FF","bold":true,"click_event":{"action":"open_url","url":"https://example.com"}}"##,
            r#"{"text":"a","hover_event":{"action":"show_text","value":["tip",{"text":"x"}]}}"#,
            r#"{"translate":"k","fallback":5}"#,
            r#"{"nbt":"a","entity":"@s","interpret":"x","plain":5,"separator":5}"#,
            r#"{"translate":"k"}"#,
            r#"{"translate":"k","fallback":"f","with":["a",1,true,{"text":"b"}]}"#,
            r#"{"keybind":"key.jump"}"#,
            r#"{"selector":"@a","separator":{"text":","}}"#,
            r#"{"score":{"name":"@s","objective":"o"}}"#,
            r#"{"nbt":"a.b","entity":"@s"}"#,
            r#"{"nbt":"a.b","block":"0 64 0","interpret":true}"#,
            r#"{"nbt":"a.b","storage":"ns:s"}"#,
            r#"{"type":"nbt","source":"storage","nbt":"a","storage":"ns:s"}"#,
            r#"{"type":"text","text":"a"}"#,
            r#"{"type":"translatable","translate":"k"}"#,
            r#"{"object":"atlas","sprite":"block/stone"}"#,
            r#"{"player":{"name":"Steve"}}"#,
            r#"{"text":"a","extra":["b",{"text":"c","extra":[{"keybind":"key.use"}]}]}"#,
            r#"["a",{"text":"b"},["c"]]"#,
            "{text:a}",
            "[{text:'a'},'b']",
        ] {
            assert!(component_from_json(accepted).is_some(), "{accepted}");
        }
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn host_nbt_and_owned_stack_text_follow_component_semantics() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use mod_api::{Host, Identifier, ItemStack, Tag, Text, TextHover};
        use serde_json::json;
        use std::sync::Arc;

        let name = "game::native_text_json::tests::host_nbt_and_owned_stack_text_follow_component_semantics";
        if run_fresh_cases(name, "SUNRISE_ITEM_STACK_TEXT_CHILD", &["components"]) {
            return;
        }
        with_game(|game| {
            let models = Arc::make_mut(&mut game.pack.item_models);
            let prototypes = Arc::make_mut(&mut models.prototypes);
            prototypes.insert(
                "minecraft:stone".into(),
                [
                    (
                        "minecraft:custom_name".into(),
                        json!({"text": "Default custom", "color": "red"}),
                    ),
                    ("minecraft:item_name".into(), json!("Default item")),
                    ("minecraft:lore".into(), json!([["Default lore"]])),
                ]
                .into(),
            );
            prototypes.insert("minecraft:stick".into(), Default::default());
            let host: &dyn Host = game;

            let mut label = Text::new(TextContent::Translatable {
                key: "fixture.label".into(),
                fallback: Some("Hello %s".into()),
                arguments: vec![Text::literal("Ada")],
            })
            .append(Text::literal("!"));
            label.style.bold = Some(true);
            label.style.italic = Some(false);
            label.style.hover = Some(TextHover::Text(Box::new(Text::literal("Hint"))));
            // Synthetic expected form checked with 26.3 ComponentSerialization.CODEC and NbtOps.
            let (expected, _) = sunrise_core::nbt::parse_snbt(concat!(
                "{translate:'fixture.label',fallback:'Hello %s',with:['Ada'],",
                "italic:0b,bold:1b,hover_event:{action:'show_text',value:'Hint'},extra:['!']}"
            ))
            .unwrap();
            assert_eq!(host.text_to_nbt(&label), Ok(expected.clone()));
            assert_eq!(host.text_from_nbt(&expected), Ok(Some(label.clone())));
            let literal = Text::literal("§cLiteral");
            let literal_tag = Tag::String("§cLiteral".into());
            assert_eq!(host.text_to_nbt(&literal), Ok(literal_tag.clone()));
            assert_eq!(host.text_from_nbt(&literal_tag), Ok(Some(literal.clone())));
            assert_eq!(host.text_from_nbt(&Tag::Int(4)), Ok(None));
            let deep_tag = (0..513).fold(Tag::String("leaf".into()), |tag, _| Tag::List(vec![tag]));
            assert_eq!(host.text_from_nbt(&deep_tag), Err(TextFormError::TooDeep));
            sunrise_core::nbt::discard_tag(deep_tag);

            // A separator uses one container per parent; extra uses a compound
            // and a list. A string leaf adds none; a styled literal adds one.
            let selector = |child| {
                Text::new(TextContent::Selector {
                    pattern: "@s".into(),
                    separator: Some(Box::new(child)),
                })
            };
            let separators = (0..512).fold(Text::literal("leaf"), |child, _| selector(child));
            let encoded = host.text_to_nbt(&separators).unwrap();
            let mut at = &encoded;
            for _ in 0..512 {
                assert_eq!(at.get("selector"), Some(&Tag::String("@s".into())));
                at = at.get("separator").unwrap();
            }
            assert_eq!(at, &Tag::String("leaf".into()));
            assert!(host.text_from_nbt(&encoded).unwrap().is_some());
            assert_eq!(
                host.text_to_nbt(&selector(separators)),
                Err(TextFormError::TooDeep)
            );
            let over = Tag::compound([
                ("selector", Tag::String("@s".into())),
                ("separator", encoded),
            ]);
            assert_eq!(host.text_from_nbt(&over), Err(TextFormError::TooDeep));
            sunrise_core::nbt::discard_tag(over);

            let extras =
                |leaf| (0..256).fold(leaf, |child, _| Text::literal("parent").append(child));
            let mut encoded = host.text_to_nbt(&extras(Text::literal("leaf"))).unwrap();
            assert!(host.text_from_nbt(&encoded).unwrap().is_some());
            let mut at = &mut encoded;
            for _ in 0..256 {
                let Some(Tag::List(children)) = at.get_mut("extra") else {
                    panic!("extra list missing");
                };
                at = &mut children[0];
            }
            assert_eq!(at, &Tag::String("leaf".into()));
            *at = Tag::compound([("text", Tag::String("leaf".into())), ("bold", Tag::Byte(1))]);
            assert_eq!(host.text_from_nbt(&encoded), Err(TextFormError::TooDeep));
            sunrise_core::nbt::discard_tag(encoded);
            let mut leaf = Text::literal("leaf");
            leaf.style.bold = Some(true);
            let too_deep = extras(leaf);
            assert_eq!(host.text_to_nbt(&too_deep), Err(TextFormError::TooDeep));

            let mut default_custom_name = Text::literal("Default custom");
            default_custom_name.style.color = Some("red".into());
            for (id, defaults) in [("minecraft:stone", true), ("minecraft:stick", false)] {
                let key = host.item_key(&Identifier::parse(id).unwrap()).unwrap();
                let mut stack = ItemStack::of(key);
                assert_eq!(
                    stack.custom_name(host),
                    Ok(defaults.then(|| default_custom_name.clone()))
                );
                assert_eq!(
                    stack.item_name(host),
                    Ok(defaults.then(|| Text::literal("Default item")))
                );
                let mut lore = stack.lore(host).unwrap();
                assert_eq!(
                    lore,
                    if defaults {
                        vec![Text::literal("Default lore")]
                    } else {
                        vec![]
                    }
                );
                lore.push(literal.clone());
                stack.set_lore(host, &lore).unwrap();
                stack.set_custom_name(host, Some(&label)).unwrap();
                stack.set_item_name(host, Some(&literal)).unwrap();
                assert_eq!(stack.custom_name(host), Ok(Some(label.clone())));
                assert_eq!(stack.item_name(host), Ok(Some(literal.clone())));
                assert_eq!(stack.lore(host), Ok(lore));

                let mut opaque = Text::literal("With a payload");
                let payload = Tag::ByteArray(vec![1, 2]);
                opaque.style.click = Some(TextClick::Custom {
                    id: "fixture:event".into(),
                    payload: Some(payload.clone()),
                });
                stack.set_lore(host, &[opaque]).unwrap();
                let mut lines = stack.lore(host).unwrap();
                lines.push(literal.clone());
                stack.set_lore(host, &lines).unwrap();
                let patch = stack.components().unwrap().as_nbt();
                let Some(Tag::List(lines)) = patch.get(&"minecraft:lore".into()) else {
                    panic!("lore list missing");
                };
                assert_eq!(
                    lines[0]
                        .get("click_event")
                        .and_then(|event| event.get("payload")),
                    Some(&payload)
                );

                let before = stack.clone();
                let components = Tag::compound([(
                    "minecraft:custom_name",
                    Tag::compound([("selector", Tag::String("@".into()))]),
                )]);
                let mut malformed_hover = Text::literal("Malformed hover item");
                malformed_hover.style.hover = Some(TextHover::Item {
                    id: "minecraft:stone".into(),
                    count: 1,
                    components: Some(components),
                });
                assert_eq!(
                    stack.set_custom_name(host, Some(&malformed_hover)),
                    Err(TextFormError::Malformed)
                );
                assert_eq!(stack, before);

                let mut malformed = Text::literal("Malformed");
                malformed.style.color = Some("not_a_color".into());
                assert_eq!(
                    stack.set_custom_name(host, Some(&malformed)),
                    Err(TextFormError::Malformed)
                );
                assert_eq!(
                    stack.set_item_name(host, Some(&malformed)),
                    Err(TextFormError::Malformed)
                );
                assert_eq!(
                    stack.set_lore(host, &[literal.clone(), malformed]),
                    Err(TextFormError::Malformed)
                );
                let invalid = [label.clone(), too_deep.clone()];
                assert_eq!(stack.set_lore(host, &invalid), Err(TextFormError::TooDeep));
                assert_eq!(
                    stack.set_custom_name(host, Some(&too_deep)),
                    Err(TextFormError::TooDeep)
                );
                assert_eq!(
                    stack.set_item_name(host, Some(&too_deep)),
                    Err(TextFormError::TooDeep)
                );
                assert_eq!(
                    stack.set_lore(host, &vec![literal.clone(); 257]),
                    Err(TextFormError::Malformed)
                );
                assert_eq!(stack, before);
                let full = vec![literal.clone(); 256];
                stack.set_lore(host, &full).unwrap();
                assert_eq!(stack.lore(host), Ok(full));

                let restore_defaults = |stack: &mut ItemStack| {
                    let mut equivalent = Text::literal("Default custom");
                    equivalent.style.color = Some("#FF5555".into());
                    stack.set_custom_name(host, Some(&equivalent)).unwrap();
                    assert_eq!(
                        stack.custom_name(host),
                        Ok(Some(default_custom_name.clone()))
                    );
                    stack
                        .set_item_name(host, Some(&Text::literal("Default item")))
                        .unwrap();
                    stack
                        .set_lore(host, &[Text::literal("Default lore")])
                        .unwrap();
                    assert!(stack.components().is_none());
                };
                if defaults {
                    restore_defaults(&mut stack);
                }
                stack.set_custom_name(host, None).unwrap();
                stack.set_item_name(host, None).unwrap();
                stack.set_lore(host, &[]).unwrap();
                assert_eq!(stack.custom_name(host), Ok(None));
                assert_eq!(stack.item_name(host), Ok(None));
                assert_eq!(stack.lore(host), Ok(vec![]));
                if defaults {
                    let patch = stack.components().unwrap().as_nbt();
                    assert_eq!(patch.len(), 3);
                    for id in [
                        "!minecraft:custom_name",
                        "!minecraft:item_name",
                        "!minecraft:lore",
                    ] {
                        assert_eq!(
                            patch.get(&id.into()),
                            Some(&Tag::Compound(Default::default()))
                        );
                    }
                    restore_defaults(&mut stack);
                } else {
                    assert!(stack.components().is_none());
                }
            }
        });
    }

    /// Through the public host: literal valid components read, literal invalid ones are
    /// `Ok(None)`, and a text over the nesting limit is the named refusal.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn the_host_reads_components_and_returns_none_for_the_rest() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let host: &dyn mod_api::Host = game;
            let read = |json: &str| {
                host.text_from_json(json)
                    .unwrap()
                    .map(|t| t.content.clone())
            };
            assert_eq!(
                read(r#"{"text":"Click","click_event":{"action":"run_command","value":"/help"}}"#),
                None
            );
            assert_eq!(
                read(r#"{"text":"Hi"}"#),
                Some(TextContent::Literal("Hi".into()))
            );
            assert_eq!(
                read("\"plain\""),
                Some(TextContent::Literal("plain".into()))
            );
            assert_eq!(
                read(r#"["a",{"text":"b"}]"#),
                Some(TextContent::Literal("a".into()))
            );
            assert!(matches!(
                read(r#"{"translate":"chat.type.text"}"#),
                Some(TextContent::Translatable { .. })
            ));
            // An explicit `type` decides alone: the stray `text` and the other sources are ignored.
            assert_eq!(
                read(r#"{"type":"keybind","keybind":"key.jump","text":"wrong"}"#),
                Some(TextContent::Keybind("key.jump".into()))
            );
            assert!(matches!(
                read(r#"{"type":"nbt","source":"entity","nbt":"Health","block":"0 64 0","entity":"@s"}"#),
                Some(TextContent::Nbt { path, source: TextNbtSource::Entity(selector), .. })
                    if path == "Health" && selector == "@s"
            ));
            let hover = host.text_from_json(r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","keybind":"key.jump","text":"wrong"}}}"#).unwrap().unwrap();
            assert!(
                matches!(&hover.style.hover, Some(mod_api::text::TextHover::Entity { name: Some(name), .. }) if name.content == TextContent::Keybind("key.jump".into()))
            );
            assert_eq!(
                read(
                    r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","text":"wrong"}}}"#
                ),
                None
            );
            for valid in [
                r#"{"text":"a","bold":true}"#,
                r#"{"text":"a","italic":false}"#,
                r#"{"text":"a","underlined":true}"#,
                r#"{"text":"a","strikethrough":false}"#,
                r#"{"text":"a","obfuscated":true}"#,
            ] {
                assert!(read(valid).is_some(), "{valid}");
            }
            for invalid in NOT_COMPONENTS.into_iter().chain(NUMERIC_FLAGS) {
                assert_eq!(read(invalid), None, "{invalid}");
            }
            assert_eq!(
                read(r#"{"text":"\u0041"}"#),
                Some(TextContent::Literal("A".into()))
            );
            let mut deep = String::from(r#""leaf""#);
            for _ in 0..513 {
                deep = format!(r#"{{"text":"n","extra":[{deep}]}}"#);
            }
            assert_eq!(
                host.text_from_json(&deep)
                    .map(|text| text.map(|t| t.content.clone())),
                Err(mod_api::text::TextFormError::TooDeep)
            );
            assert_eq!(read("{} trailing"), None);
            assert_eq!(read("12"), None);
        });
    }
}
