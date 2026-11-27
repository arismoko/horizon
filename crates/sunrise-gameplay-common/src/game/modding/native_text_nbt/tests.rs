use super::*;
use mod_api::text::{TextClick, TextHover, TextProfile};

fn codec(registries: &Registries) -> Codec<'_> {
    Codec {
        registries,
        defer_holders: false,
    }
}

fn snbt(input: &str) -> Tag {
    let (tag, used) = nbt::parse_snbt(input).unwrap();
    assert_eq!(used, input.len());
    tag
}

#[test]
fn custom_click_retains_typed_nbt_even_when_unrelated_fields_are_non_finite() {
    let reg = Registries::default();
    let codec = codec(&reg);
    // Build the non-finite tag directly: it is an NBT value, not JSON or SNBT syntax.
    let payload = Tag::compound([
        ("bytes", Tag::ByteArray(vec![1, 2])),
        ("ints", Tag::IntArray(vec![3, 4])),
        ("float", Tag::Float(f32::INFINITY.to_bits())),
    ]);
    let click = Tag::compound([
        ("action", s("custom")),
        ("id", s("test:event")),
        ("payload", nbt::clone_tag(&payload)),
    ]);
    let supplied = Tag::compound([
        ("text", s("Example")),
        ("click_event", nbt::clone_tag(&click)),
        ("unknown", Tag::Double(f64::NAN.to_bits())),
    ]);
    let mut written = Text::literal("Example");
    written.style.click = Some(TextClick::Custom {
        id: "test:event".into(),
        payload: Some(payload),
    });
    assert_eq!(codec.read(&supplied), Ok(Some(written.clone())));
    let expected = Tag::compound([("text", s("Example")), ("click_event", click)]);
    assert_eq!(codec.write(&written), Ok(expected));
}

#[test]
fn click_payload_depth_includes_the_binary_mixed_list_wrapper() {
    let reg = Registries::default();
    let codec = codec(&reg);
    let with_chain = |chain_len| {
        let chain = (0..chain_len).fold(s("leaf"), |tag, _| Tag::List(vec![tag]));
        let mut text = Text::literal("Click");
        text.style.click = Some(TextClick::Custom {
            id: "test:event".into(),
            payload: Some(Tag::List(vec![s("first"), chain])),
        });
        text
    };
    // Component and event compounds, mixed list, synthetic compound, and 508 lists.
    let text = with_chain(508);
    let tag = codec.write(&text).unwrap();
    let limits = Limits::default();
    let bytes = nbt::write_network(&tag, limits).unwrap();
    let (decoded, used) = nbt::read_network(&bytes, limits).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(codec.read(&decoded), Ok(Some(text)));
    nbt::discard_tag(tag);
    nbt::discard_tag(decoded);

    assert_eq!(codec.write(&with_chain(509)), Err(Error::TooDeep));
}

#[test]
fn hover_item_components_use_the_persistent_text_codec() {
    let reg = Registries::default();
    let codec = codec(&reg);
    let components = snbt("{'minecraft:custom_name':{player:{name:'Alex',texture:'skin'}}}");
    let expected_patch =
        snbt("{'minecraft:custom_name':{player:{name:'Alex',texture:'minecraft:skin'}}}");
    let hover_tag = |components| {
        Tag::compound([
            ("text", s("Item")),
            (
                "hover_event",
                Tag::compound([
                    ("action", s("show_item")),
                    ("id", s("minecraft:stone")),
                    ("components", components),
                ]),
            ),
        ])
    };
    let mut text = Text::literal("Item");
    text.style.hover = Some(TextHover::Item {
        id: "minecraft:stone".into(),
        count: 1,
        components: Some(components.clone()),
    });
    assert_eq!(codec.write(&text), Ok(hover_tag(expected_patch.clone())));
    let decoded = codec.read(&hover_tag(components)).unwrap().unwrap();
    assert_eq!(
        decoded.style.hover,
        Some(TextHover::Item {
            id: "minecraft:stone".into(),
            count: 1,
            components: Some(expected_patch),
        })
    );
    let invalid = snbt("{'minecraft:custom_name':{selector:'@'}}");
    assert_eq!(codec.read(&hover_tag(invalid)), Ok(None));
}

#[test]
fn nbt_flags_use_numbers_without_changing_json_booleans() {
    let reg = Registries::default();
    let codec = codec(&reg);
    let input = snbt("{text:'flag',bold:0.5f,italic:-0.0d}");
    let text = codec.read(&input).unwrap().unwrap();
    assert_eq!(
        (text.style.bold, text.style.italic),
        (Some(true), Some(false))
    );
    assert_eq!(
        codec.write(&text),
        Ok(snbt("{text:'flag',bold:1b,italic:0b}"))
    );
    assert_eq!(codec.read(&snbt("{text:'flag',bold:'true'}")), Ok(None));
    assert!(
        sunrise_sim::text_grammar::build(
            &serde_json::json!({"text":"flag","bold":0.5}),
            &reg,
            false
        )
        .is_none()
    );

    let input = snbt("{nbt:'value',storage:'test:data',interpret:0.0f,plain:0.5f}");
    let text = codec.read(&input).unwrap().unwrap();
    assert_eq!(
        codec.write(&text),
        Ok(snbt("{nbt:'value',storage:'test:data',plain:1b}"))
    );
    assert_eq!(
        codec.read(&snbt(
            "{nbt:'value',storage:'test:data',interpret:0.5f,plain:0.25d}"
        )),
        Ok(None)
    );
    let player = codec
        .read(&snbt("{player:'Alex',hat:0.5f}"))
        .unwrap()
        .unwrap();
    assert_eq!(codec.write(&player), Ok(snbt("{player:{name:'Alex'}}")));
}

#[test]
fn compact_output_uses_the_persistent_field_defaults() {
    let reg = Registries::default();
    let codec = codec(&reg);
    // Synthetic outputs checked using 26.3 ComponentSerialization.CODEC and NbtOps.
    let atlas = Text::new(TextContent::Object(TextObject::Atlas {
        atlas: "minecraft:blocks".into(),
        sprite: "test:icon".into(),
        fallback: None,
    }));
    let mut player_profile = TextProfile::default();
    player_profile.name = Some("Alex".into());
    let player = Text::new(TextContent::Object(TextObject::Player {
        profile: player_profile,
        hat: true,
        fallback: None,
    }));
    let mut item = Text::literal("item");
    item.style.hover = Some(TextHover::Item {
        id: "stone".into(),
        count: 1,
        components: Some(Tag::Compound(Compound::new())),
    });
    let nbt = Text::new(TextContent::Nbt {
        path: "value".into(),
        interpret: false,
        source: TextNbtSource::Storage("test:data".into()),
        separator: None,
    });
    let translatable = Text::translate("fixture", vec![Text::literal("42")]);
    for (text, expected) in [
        (atlas, "{sprite:'test:icon'}"),
        (player, "{player:{name:'Alex'}}"),
        (
            item,
            "{text:'item',hover_event:{action:show_item,id:'minecraft:stone'}}",
        ),
        (nbt, "{nbt:'value',storage:'test:data'}"),
        (translatable, "{translate:'fixture',with:['42']}"),
    ] {
        assert_eq!(codec.write(&text), Ok(snbt(expected)));
    }
    let input = snbt("{translate:'fixture',with:[42s,42,42L,0.5f,1.25d,'42',{text:'42'}]}");
    let decoded = codec.read(&input).unwrap().unwrap();
    assert_eq!(
        codec.write(&decoded),
        Ok(snbt(
            "{translate:'fixture',with:[42s,42,42L,0.5f,1.25d,'42','42']}"
        ))
    );
}
