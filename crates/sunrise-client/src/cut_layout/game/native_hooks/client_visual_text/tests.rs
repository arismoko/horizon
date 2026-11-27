#![cfg(feature = "client")]
use mod_api::{ClientHost, Text, TextAlign, TextLayoutSpec, VisualTextCharacter};

fn retain_visual(host: &dyn ClientHost, text: &Text) -> Vec<VisualTextCharacter> {
    let mut output = Vec::new();
    assert!(host.visit_text_visual_order(text, &mut |character| {
        output.push(character.to_owned());
        true
    }));
    output
}

#[test]
#[ignore = "daily"]
fn visual_read_uses_live_translations_styles_and_bidi_outside_drawing() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::client_visual_text::tests::visual_read_uses_live_translations_styles_and_bidi_outside_drawing",
        || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                check_visual_reads(game)
            })
        },
    );
}

fn check_visual_reads(host: &dyn ClientHost) {
    let mut argument = Text::literal("Alex");
    argument.style.bold = Some(true);
    argument.style.color = Some("red".into());
    let mut line = Text::translate("fixture.greet", vec![argument]);
    line.style.color = Some("gold".into());
    sunrise_game_data::lang::scoped(
        sunrise_game_data::lang::Language::of(
            "test",
            [("fixture.greet".into(), "Hello, %s!".into())],
        ),
        || {
            let output = retain_visual(host, &line);
            assert_eq!(
                output
                    .iter()
                    .map(|v| char::from_u32(v.scalar).unwrap())
                    .collect::<String>(),
                "Hello, Alex!"
            );
            assert_eq!(
                output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
                [0, 1, 2, 3, 4, 5, 6, 0, 1, 2, 3, 0]
            );
            assert!(
                output[7..11]
                    .iter()
                    .all(|v| v.style.bold == Some(true)
                        && v.style.color.as_deref() == Some("#FF5555"))
            );
            assert!(
                output[..7]
                    .iter()
                    .chain(&output[11..])
                    .all(|v| v.style.color.as_deref() == Some("#FFAA00"))
            );
        },
    );
    for (input, expected, indices) in [
        ("שלום", "םולש", vec![3, 2, 1, 0]),
        (
            "abc אבג 123",
            "abc 123 גבא",
            vec![0, 1, 2, 3, 0, 1, 2, 3, 2, 1, 0],
        ),
    ] {
        let output = retain_visual(host, &Text::literal(input));
        assert_eq!(
            output
                .iter()
                .map(|v| char::from_u32(v.scalar).unwrap())
                .collect::<String>(),
            expected
        );
        assert_eq!(
            output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
            indices
        );
    }
    // Independent equal styles form one UTF-16 run; a color or click change starts
    // another. The sidecar must still return each character's own opaque metadata.
    let mut dialog = mod_api::Tag::compound([("leaf", mod_api::Tag::Int(7))]);
    for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
        dialog = mod_api::Tag::compound([("next", dialog)]);
    }
    let mut separate = Text::literal("");
    for (letter, color, marker) in [
        ("A", "red", 1),
        ("B", "#FF5555", 2),
        ("C", "gold", 3),
        ("D", "gold", 4),
    ] {
        let mut child = Text::literal(letter);
        child.style.color = Some(color.into());
        child.style.metadata = mod_api::Tag::compound([("marker", mod_api::Tag::Int(marker))]);
        child.style.click = Some(if marker == 4 {
            mod_api::text::TextClick::CopyToClipboard("different".into())
        } else {
            mod_api::text::TextClick::ShowDialog(sunrise_core::nbt::clone_tag(&dialog))
        });
        separate.children.push(child);
    }
    let output = retain_visual(host, &separate);
    assert_eq!(
        output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
        [0, 1, 0, 0]
    );
    for (i, output) in output.iter().enumerate() {
        assert_eq!(
            output.style.metadata.get("marker"),
            Some(&mod_api::Tag::Int(i as i32 + 1))
        );
    }
    sunrise_core::nbt::discard_tag(dialog);

    let mut custom = Text::literal("");
    for (letter, id) in [
        ("A", "example"),
        ("B", "minecraft:example"),
        ("C", ":example"),
        ("D", "other:example"),
    ] {
        let mut child = Text::literal(letter);
        child.style.click = Some(mod_api::text::TextClick::Custom {
            id: id.into(),
            payload: Some(mod_api::Tag::Int(7)),
        });
        custom.children.push(child);
    }
    let output = retain_visual(host, &custom);
    assert_eq!(
        output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
        [0, 1, 2, 0]
    );
    for (character, input) in output.iter().zip(&custom.children) {
        assert_eq!(character.style.click, input.style.click);
    }

    let mut styled = Text::literal("X");
    styled.style.color = Some("#123456".into());
    styled.style.bold = Some(false);
    styled.style.italic = Some(true);
    styled.style.underlined = Some(true);
    styled.style.strikethrough = Some(true);
    styled.style.obfuscated = Some(true);
    styled.style.shadow_color = Some(0x80123456);
    styled.style.font = Some("minecraft:uniform".into());
    styled.style.insertion = Some("insert".into());
    styled.style.click = Some(mod_api::text::TextClick::CopyToClipboard("copy".into()));
    styled.style.hover = Some(mod_api::TextHover::Text(Box::new(Text::literal("hover"))));
    let output = retain_visual(host, &styled);
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].style, styled.style);
    assert!(retain_visual(host, &Text::literal("")).is_empty());
    // In-process visual order accepts deep trees (matching vanilla Component.getVisualOrderText).
    let mut deep = Text::literal("A");
    for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
        let mut parent = Text::literal("");
        parent.children.push(deep);
        deep = parent;
    }
    let deep_output = retain_visual(host, &deep);
    assert_eq!(deep_output.len(), 1);
    assert_eq!(deep_output[0].scalar, 'A' as u32);
    let mut early = Vec::new();
    assert!(
        !host.visit_text_visual_order(&Text::literal("A😀BC"), &mut |v| {
            early.push((char::from_u32(v.scalar).unwrap(), v.source_index));
            early.len() < 3
        })
    );
    assert_eq!(early, [('A', 0), ('😀', 1), ('B', 3)]);

    // Neither early stop nor advancing to the next style may recursively free opaque tags.
    let mut nested = mod_api::Tag::Int(7);
    for _ in 0..32_768 {
        nested = mod_api::Tag::List(vec![nested]);
    }
    let mut payload = Text::literal("X");
    payload.style.metadata = mod_api::Tag::compound([("deep", nested)]);
    let payload = Text::literal("").append(payload).append(Text::literal("Y"));
    let mut count = 0;
    let mut retained = None;
    assert!(!host.visit_text_visual_order(&payload, &mut |v| {
        count += 1;
        retained = Some(v.to_owned());
        false
    }));
    assert_eq!(count, 1);
    let retained = retained.unwrap();
    assert_eq!(retained.scalar, 'X' as u32);
    let mut inner = retained.style.metadata.get("deep").unwrap();
    for _ in 0..32_768 {
        let mod_api::Tag::List(layer) = inner else {
            panic!("retained style lost a list layer");
        };
        inner = &layer[0];
    }
    assert_eq!(inner, &mod_api::Tag::Int(7));
    drop(retained);
    assert!(host.visit_text_visual_order(&payload, &mut |_| {
        count += 1;
        true
    }));
    assert_eq!(count, 3);

    let spec = TextLayoutSpec {
        max_width: 100.0,
        align: TextAlign::Left,
        scale: 2.0,
    };
    let mut retained = Vec::new();
    let size = host
        .visit_text_lines(&Text::literal("One\nTwo"), spec, &mut |line| {
            retained.push((line.text.clone(), line.at, line.width));
        })
        .unwrap();
    assert_eq!(retained.len(), 2);
    for (i, word) in ["One", "Two"].into_iter().enumerate() {
        let (line, at, width) = &retained[i];
        let resolved = sunrise_client_lib::api_text::to_engine(line).unwrap();
        assert_eq!(resolved.plain(), word);
        assert_eq!(*at, [0.0, i as f32 * 18.0]);
        assert_eq!(*width, host.text_width(&Text::literal(word)) as f32 * 2.0);
    }
    assert_eq!(size.height, 36.0);
    assert_eq!(size.width, retained[0].2.max(retained[1].2));
}
