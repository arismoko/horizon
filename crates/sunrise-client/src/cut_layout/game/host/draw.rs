#![cfg(test)]
use mod_api::Tag;
use mod_api::Text;
use sunrise_client_gameplay::game::host::draw::GameDrawContext;
#[cfg(test)]
pub(crate) mod tests {
    use mod_api::{DrawContext, TooltipFont};
    use mod_api::{
        Identifier, TextAlign, TextFormError, TextLayoutError, TextLayoutSize, TextLayoutSpec,
    };
    use std::collections::BTreeMap;
    use sunrise_client_gameplay::game::native_hooks::TryIntoEngine;
    use {
        crate::game::host::draw::*,
        sunrise_client_gameplay::game::host::draw::{
            EngineTooltipFont, canvas_test_font, test_draw_gui_canvas, text_layout,
        },
    };

    /// A font whose printable characters are all `advance` GUI pixels wide: the vanilla provider
    /// list with a `space` provider, so a layout test measures exact advances.
    fn fake_font(advance: f32) -> crate::font::Font {
        let advances: Vec<String> = (32u8..=126)
            .map(char::from)
            // A JSON object key: the quote and the backslash are the two that need escaping.
            .filter(|&c| !matches!(c, '"' | '\\'))
            .map(|c| format!("\"{c}\":{advance}"))
            .collect();
        let json = format!(
            r#"{{"providers":[{{"type":"space","advances":{{{}}}}}]}}"#,
            advances.join(",")
        );
        let source = sunrise_assets::resource_source::ResourceSource::indexed(BTreeMap::from([(
            "assets/minecraft/font/default.json".into(),
            sunrise_assets::resource_source::Resource::bytes(json.into_bytes()),
        )]));
        crate::font::Font::metrics(&source)
    }

    /// A line's component as the game draws it.
    fn engine(text: &mod_api::Text) -> sunrise_game_data::text::Text {
        text.try_into_engine(&()).unwrap()
    }

    #[derive(Debug)]
    struct MeasuredLine {
        text: Text,
        at: [f32; 2],
        width: f32,
    }

    #[derive(Debug)]
    struct Measurement {
        lines: Vec<MeasuredLine>,
        size: TextLayoutSize,
    }

    fn measured(
        font: &crate::font::Font,
        text: &Text,
        max_width: f32,
        align: TextAlign,
        scale: f32,
    ) -> Result<Measurement, TextLayoutError> {
        let layout = text_layout::LabelLayout::try_from(TextLayoutSpec {
            max_width,
            align,
            scale,
        })?;
        let mut lines = Vec::new();
        let size = layout.visit_components(font, text, &mut |line| {
            lines.push(MeasuredLine {
                text: line.text.clone(),
                at: line.at,
                width: line.width,
            })
        })?;
        Ok(Measurement { lines, size })
    }

    /// The lines' plain characters, top to bottom.
    fn plain(lines: &Measurement) -> Vec<String> {
        lines
            .lines
            .iter()
            .map(|line| engine(&line.text).plain())
            .collect()
    }

    /// Each character six GUI pixels wide, as the widget tests fake one.
    fn layout(text: &str, max_width: f32, align: TextAlign, scale: f32) -> Measurement {
        measured(
            &fake_font(6.0),
            &Text::literal(text),
            max_width,
            align,
            scale,
        )
        .unwrap()
    }

    #[test]
    fn a_label_wraps_at_spaces_and_keeps_its_words() {
        // 48 pixels is eight characters: "a quick" fits, "brown" does not fit after it, and
        // "brown fox" is one character too wide for the second line.
        let lines = layout("a quick brown fox", 48.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["a quick", "brown", "fox"]);
        assert_eq!(lines.size.width, 42.0);
        assert!(
            lines.lines.iter().all(|line| line.width <= 48.0),
            "{:?}",
            lines.lines.iter().map(|l| l.width).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_at_a_character() {
        let lines = layout("abcdefghij", 24.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["abcd", "efgh", "ij"]);
        assert_eq!(plain(&lines).concat(), "abcdefghij");
    }

    #[test]
    fn an_explicit_newline_starts_a_line_and_an_empty_component_has_no_line() {
        assert_eq!(
            plain(&layout("AA\nBB", 100.0, TextAlign::Left, 1.0)),
            ["AA", "BB"]
        );
        let empty = layout("", 100.0, TextAlign::Left, 1.0);
        assert!(empty.lines.is_empty());
        assert_eq!((empty.size.width, empty.size.height), (0.0, 0.0));
    }

    #[test]
    fn a_trailing_newline_leaves_empty_last_lines() {
        // One `\n` ends the line it closes and the label still holds the empty line after it;
        // two leave two, the second from the split's own trailing newline.
        let one = layout("AA\n", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&one), ["AA", ""]);
        assert_eq!((one.size.width, one.size.height), (12.0, 18.0));
        let two = layout("AA\n\n", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&two), ["AA", "", ""]);
        assert_eq!((two.size.width, two.size.height), (12.0, 27.0));
    }

    #[test]
    fn leading_spaces_and_runs_of_spaces_stay_in_the_line() {
        // Nothing trims a label: the spaces at its start are characters like any other.
        let lead = layout("  AA", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lead), ["  AA"]);
        assert_eq!(lead.size.width, 24.0);
        // A run between words is kept when it fits, and the space a wrap breaks at is dropped.
        assert_eq!(
            plain(&layout("AA   BB", 100.0, TextAlign::Left, 1.0)),
            ["AA   BB"]
        );
        assert_eq!(
            plain(&layout("  AA BB", 24.0, TextAlign::Left, 1.0)),
            ["  AA", "BB"]
        );
    }

    #[test]
    fn spaces_at_a_break_leave_empty_lines() {
        // A leading space that does not fit is the break: an empty first line, then one character
        // a line. The break at a run of spaces drops only the space it breaks at.
        assert_eq!(
            plain(&layout(" AA", 6.0, TextAlign::Left, 1.0)),
            ["", "A", "A"]
        );
        assert_eq!(
            plain(&layout("AA    BBBB", 24.0, TextAlign::Left, 1.0)),
            ["AA  ", "", "BBBB"]
        );
    }

    #[test]
    fn each_line_is_placed_by_the_alignment() {
        // 24 and 12 pixels wide, in a box of 100: left leaves them at 0, centre at 38 and 44,
        // right at 76 and 88.
        for (align, expected) in [
            (TextAlign::Left, [0.0, 0.0]),
            (TextAlign::Centre, [38.0, 44.0]),
            (TextAlign::Right, [76.0, 88.0]),
        ] {
            let lines = layout("AAAA\nBB", 100.0, align, 1.0);
            let placed: Vec<f32> = lines.lines.iter().map(|line| line.at[0]).collect();
            assert_eq!(placed, expected, "{align:?}");
        }
    }

    #[test]
    fn a_centred_line_rounds_the_half_line_width_down() {
        // A 15-pixel line in a 40-pixel box: `TextAlignment.CENTER.calculateLeft` floors the half
        // line width, so it starts at 13, not at the exact 12.5 an unrounded centre would give.
        let lines = measured(
            &fake_font(5.0),
            &Text::literal("AAA"),
            40.0,
            TextAlign::Centre,
            1.0,
        )
        .unwrap();
        assert_eq!(lines.lines[0].at[0], 13.0);
    }

    #[test]
    fn the_height_is_the_line_count_times_the_line_height() {
        let lines = layout("A\nB\nC", 100.0, TextAlign::Left, 1.0);
        let down: Vec<f32> = lines.lines.iter().map(|line| line.at[1]).collect();
        assert_eq!(down, [0.0, 9.0, 18.0]);
        assert_eq!(lines.size.height, 3.0 * 9.0);
        // A viewport sizes itself from the last line rather than the count.
        assert_eq!(lines.size.height, down.last().unwrap() + 9.0);
    }

    #[test]
    fn styles_survive_wrapping() {
        let mut text = mod_api::Text::literal("AAAA BBBB");
        text.style.bold = Some(true);
        let lines = measured(&fake_font(6.0), &text, 30.0, TextAlign::Left, 1.0).unwrap();
        assert_eq!(plain(&lines), ["AAAA", "BBBB"]);
        for line in &lines.lines {
            let line = engine(&line.text);
            assert!(
                line.chars().all(|(c, style)| c == ' ' || style.is_bold()),
                "{:?}",
                line.plain()
            );
        }
    }

    #[test]
    fn laid_out_objects_keep_their_sprites_profiles_and_fallbacks_on_redraw() {
        use mod_api::text::{TextContent, TextObject, TextProfile};
        use sunrise_game_data::text::InlineObject;

        let atlas = Text::new(TextContent::Object(TextObject::Atlas {
            atlas: "minecraft:gui".into(),
            sprite: "minecraft:heart/full".into(),
            fallback: Some(Box::new(Text::literal("[heart]"))),
        }));
        let mut profile = TextProfile::default();
        profile.name = Some("Alex".into());
        let mut head = Text::new(TextContent::Object(TextObject::Player {
            profile,
            hat: false,
            fallback: Some(Box::new(Text::literal("[Alex]"))),
        }));
        head.style.shadow_color = Some(0xFF00_0011);
        let input = atlas.append(head);
        let font = fake_font(6.0);
        let lines = measured(&font, &input, 150.0, TextAlign::Left, 1.0).unwrap();
        let redrawn = engine(&lines.lines[0].text);
        assert_eq!(redrawn.plain(), "[heart][Alex]");
        assert_eq!(font.width_text(&redrawn), lines.lines[0].width);
        let [atlas, head] = redrawn.spans.as_slice() else {
            panic!("expected two object spans")
        };
        assert!(matches!(&atlas.style.inline_object,
            Some(InlineObject::Atlas { atlas, sprite, .. })
            if atlas == "minecraft:gui" && sprite == "minecraft:heart/full"));
        assert!(matches!(&head.style.inline_object,
            Some(InlineObject::Player { profile, hat: false, .. })
            if profile.get("name").and_then(serde_json::Value::as_str) == Some("Alex")));
        assert_eq!(head.style.shadow_color, Some(0xFF00_0011));
    }

    #[test]
    fn layout_does_not_read_style_indices_from_mod_metadata() {
        use mod_api::{Tag, text::TextClick};
        let mut styled = Text::literal("A");
        styled.style.click = Some(TextClick::CopyToClipboard("not for B".into()));
        let mut plain = Text::literal("B");
        plain.metadata = Tag::compound([("__sunrise_local_style_id", Tag::Int(0))]);
        let input = Text::literal("").append(styled).append(plain);
        let lines = measured(&fake_font(6.0), &input, 150.0, TextAlign::Left, 1.0).unwrap();
        let [styled, plain] = lines.lines[0].text.children.as_slice() else {
            panic!("expected two styles")
        };
        assert_eq!(
            styled.style.click,
            Some(TextClick::CopyToClipboard("not for B".into()))
        );
        assert_eq!(
            plain.content,
            mod_api::text::TextContent::Literal("B".into())
        );
        assert_eq!(plain.style.click, None);
    }

    #[test]
    fn layout_copies_inherited_deep_hover_and_opaque_metadata_without_recursion() {
        use mod_api::{
            Tag,
            text::{TextContent, TextHover},
        };
        const DEPTH: usize = 4096;
        let mut hover = Text::literal("hover leaf");
        let mut metadata = Tag::Int(73);
        for _ in 0..DEPTH {
            let mut parent = Text::literal("");
            parent.style.hover = Some(TextHover::Text(Box::new(hover)));
            hover = parent;
            metadata = Tag::List(vec![metadata]);
        }
        let mut root = Text::literal("");
        root.style.hover = Some(TextHover::Text(Box::new(hover)));
        root.style.metadata = Tag::compound([("deep", metadata), ("override", Tag::Int(1))]);
        let mut child = Text::literal("A");
        child.style.metadata = Tag::compound([("override", Tag::Int(2))]);
        root.children.push(child);
        let lines = measured(&fake_font(6.0), &root, 150.0, TextAlign::Left, 1.0).unwrap();
        let style = &lines.lines[0].text.style;
        let Some(TextHover::Text(hover)) = &style.hover else {
            panic!("inherited hover")
        };
        let mut hover = hover.as_ref();
        let Tag::Compound(metadata) = &style.metadata else {
            panic!("inherited metadata")
        };
        assert_eq!(
            metadata[&sunrise_core::nbt::JavaString::from("override")],
            Tag::Int(2)
        );
        let mut deep = &metadata[&sunrise_core::nbt::JavaString::from("deep")];
        for _ in 0..DEPTH {
            let Tag::List(list) = deep else {
                panic!("nested metadata")
            };
            deep = &list[0];
        }
        assert_eq!(deep, &Tag::Int(73));
        for _ in 0..DEPTH {
            let Some(TextHover::Text(child)) = &hover.style.hover else {
                panic!("nested hover")
            };
            hover = child;
        }
        assert_eq!(hover.content, TextContent::Literal("hover leaf".into()));
    }

    #[test]
    fn a_reset_after_a_break_restores_the_style_the_tail_carried() {
        // `§lAA §rB` in a 14-pixel box: two bold characters fill the first line, the space is
        // where it breaks, and the second line starts in the bold style the split carried over.
        // Its `§r` goes back to that carried style, so `B` stays bold, and no line keeps a code.
        let lines = layout("§lAA §rB", 14.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["AA", "B"]);
        let bold: Vec<bool> = engine(&lines.lines[1].text)
            .chars()
            .map(|(_, style)| style.is_bold())
            .collect();
        assert_eq!(bold, [true], "the reset restores the carried bold style");
        for line in plain(&lines) {
            assert!(!line.contains('§'), "a line carries no code: {line:?}");
        }
    }

    #[test]
    fn scale_wraps_at_the_box_over_it_and_scales_every_answer() {
        // 36 pixels at scale 2 wrap in 24: two characters a line, 12 pixels measured and 24
        // answered, the lines 18 apart and the block 54 high.
        let lines = layout("AAAAAA", 24.0, TextAlign::Left, 2.0);
        assert_eq!(plain(&lines), ["AA", "AA", "AA"]);
        let widths: Vec<f32> = lines.lines.iter().map(|line| line.width).collect();
        assert_eq!(widths, [24.0, 24.0, 24.0]);
        let down: Vec<f32> = lines.lines.iter().map(|line| line.at[1]).collect();
        assert_eq!(down, [0.0, 18.0, 36.0]);
        assert_eq!((lines.size.width, lines.size.height), (24.0, 54.0));
        // Positions scale too: a 6-pixel line centred in 12 starts at 3, answered at 6.
        let one = layout("A", 24.0, TextAlign::Centre, 2.0);
        assert_eq!(one.lines[0].at, [6.0, 0.0]);
        assert_eq!(one.lines[0].width, 12.0);
    }

    #[test]
    fn a_width_below_one_still_wraps() {
        // `Font.split`'s one-pixel minimum: a box of no width, or one narrower than a pixel, gives
        // one character a line rather than an unwrapped line of any length.
        for max_width in [0.0, 0.5, -4.0] {
            let lines = layout("AAA", max_width, TextAlign::Left, 1.0);
            assert_eq!(plain(&lines), ["A", "A", "A"], "{max_width}");
            assert_eq!(lines.size.height, 27.0, "{max_width}");
        }
    }

    #[test]
    fn a_glyph_wider_than_the_box_gets_a_line_of_its_own() {
        // A 20-pixel character in a 6-pixel box: neither fits, and neither is dropped.
        let lines = measured(
            &fake_font(20.0),
            &Text::literal("AB"),
            6.0,
            TextAlign::Left,
            1.0,
        )
        .unwrap();
        assert_eq!(plain(&lines), ["A", "B"]);
        assert_eq!(lines.lines[0].width, 20.0);
        assert_eq!(lines.size.width, 20.0);
    }

    #[test]
    fn a_layout_refuses_a_width_or_a_scale_it_cannot_place() {
        for max_width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                measured(
                    &fake_font(6.0),
                    &Text::literal("AA"),
                    max_width,
                    TextAlign::Left,
                    1.0
                )
                .unwrap_err(),
                TextLayoutError::WidthNotFinite,
                "{max_width}"
            );
        }
        for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(
                measured(
                    &fake_font(6.0),
                    &Text::literal("AA"),
                    100.0,
                    TextAlign::Left,
                    scale
                )
                .unwrap_err(),
                TextLayoutError::ScaleInvalid,
                "{scale}"
            );
        }
        // A finite width `scale` makes overflow is no box at all, not a box of no width.
        assert_eq!(
            measured(
                &fake_font(6.0),
                &Text::literal("AA"),
                f32::MAX,
                TextAlign::Left,
                f32::MIN_POSITIVE
            )
            .unwrap_err(),
            TextLayoutError::WidthNotFinite
        );
    }

    #[test]
    fn a_layout_visits_every_line_and_character_without_transport_limits() {
        let font = fake_font(6.0);
        let layout = text_layout::LabelLayout::try_from(TextLayoutSpec {
            max_width: 1e9,
            align: TextAlign::Left,
            scale: 1.0,
        })
        .unwrap();
        let text = Text::literal("abcdefghijklmnop\n".repeat(600));
        let mut lines = 0;
        let mut characters = 0;
        let size = layout
            .visit_components(&font, &text, &mut |line| {
                lines += 1;
                characters += engine(line.text).plain().len();
            })
            .unwrap();
        assert_eq!(lines, 601);
        assert_eq!(characters, 9600);
        assert_eq!((size.width, size.height), (96.0, 5409.0));
    }

    #[test]
    fn warmed_wrapped_paint_lends_lines_without_allocating() {
        let font = fake_font(6.0);
        let text = sunrise_game_data::text::Text::literal("abcd ef  ghi\njkl");
        let layout = text_layout::LabelLayout::try_from(TextLayoutSpec {
            max_width: 24.0,
            align: TextAlign::Left,
            scale: 1.0,
        })
        .unwrap();
        sunrise_game_data::lang::scoped(sunrise_game_data::lang::Language::of("en_us", []), || {
            let spec = [("abcd", 0.0), ("ef ", 9.0), ("ghi", 18.0), ("jkl", 27.0)];
            let paint = || {
                let mut lines = 0;
                let size = layout
                    .visit_visual(&font, &text, |line, at| {
                        let (expected, y) = spec[lines];
                        assert!(line.chars().map(|(c, _)| c).eq(expected.chars()));
                        assert_eq!(at, [0.0, y]);
                        lines += 1;
                    })
                    .unwrap();
                assert_eq!(lines, spec.len());
                assert_eq!((size.width, size.height), (24.0, 36.0));
            };
            paint();
            let (_, allocations) = sunrise_threads::allocation_count::measure(paint);
            assert_eq!(allocations, 0);
        });
    }

    #[test]
    fn warm_item_count_and_empty_decorations_need_no_replay_allocations() {
        use crate::hud_overlays::{ItemDecorations, paint_item_decorations};
        let mut atlas = crate::hud::HudAtlas::boot();
        atlas.font = canvas_test_font();
        let mut batch = crate::hud::HudBatch::scratch([320, 240]);
        for (count, vertices) in [(1, 0), (65, 24)] {
            let decorations = ItemDecorations {
                count,
                count_text: None,
                bar: None,
                cooldown: 0.0,
            };
            paint_item_decorations(&mut batch, &atlas, [0.0; 2], &decorations);
            batch.vertices.clear();
            let (_, allocations) = sunrise_threads::allocation_count::measure(|| {
                paint_item_decorations(&mut batch, &atlas, [0.0; 2], &decorations);
            });
            assert_eq!(batch.vertices.len(), vertices);
            assert_eq!(allocations, 0, "stack count {count}");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn scaled_text_is_the_text_scaled_about_its_corner() {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let at = [10.0, 20.0];
            let mut plain = crate::hud::HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut plain).text("Jump", at, 0xFFFF_FFFF, true);
            let mut scaled = crate::hud::HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut scaled).text_scaled("Jump", at, 0.5, 0xFFFF_FFFF, true);
            assert!(!plain.vertices.is_empty(), "the text draws glyphs");
            assert_eq!(plain.vertices.len(), scaled.vertices.len());
            for (p, s) in plain.vertices.iter().zip(&scaled.vertices) {
                let half = [
                    at[0] + (p.pos[0] - at[0]) * 0.5,
                    at[1] + (p.pos[1] - at[1]) * 0.5,
                ];
                assert_eq!(s.pos, half, "every corner halfway to `at`");
                assert_eq!(
                    (s.uv, s.color),
                    (p.uv, p.color),
                    "the same glyphs and colours"
                );
            }
        });
    }

    #[test]
    fn a_wrapped_label_measures_and_draws_its_own_height() {
        let mut atlas = crate::hud::HudAtlas::boot();
        atlas.font = canvas_test_font();
        let spec = TextLayoutSpec {
            max_width: 32.0,
            align: TextAlign::Left,
            scale: 2.0,
        };
        let text = Text::literal("§cmmmm §aאבג §bmmmm");
        let mut actual = crate::hud::HudBatch::scratch([320, 240]);
        test_draw_gui_canvas(&mut actual, &atlas, None, None, &mut |canvas| {
            canvas.scope(
                &[mod_api::client::hud_layout::PoseOp::Translate(4.0, 6.0)],
                Some([5.0, 10.0, 34.0, 53.0]),
                &mut |scoped| {
                    let size =
                        scoped.text_with_word_wrap(&text, [10.0, 20.0], spec, u32::MAX, false)?;
                    assert_eq!(
                        size,
                        TextLayoutSize {
                            width: 32.0,
                            height: 54.0
                        }
                    );
                    Ok(())
                },
            )
        })
        .unwrap();

        let runs: Vec<_> = actual.scissors_in(0..66).collect();
        assert_eq!(runs, [(0..66, Some([9.0, 16.0, 43.0, 69.0]))]);
        let clip = runs[0].1.unwrap();
        // The final column and row are partly clipped, but the glyph UVs still
        // address the original cells. The Hebrew line's first glyph is gimel.
        let glyphs = [
            ([14.0, 26.0, 22.0, 42.0], [22.0, 42.0], 0.0, 0xFF55_55FF),
            ([22.0, 26.0, 30.0, 42.0], [30.0, 42.0], 0.0, 0xFF55_55FF),
            ([30.0, 26.0, 38.0, 42.0], [38.0, 42.0], 0.0, 0xFF55_55FF),
            ([38.0, 26.0, 46.0, 42.0], [43.0, 42.0], 0.0, 0xFF55_55FF),
            ([14.0, 44.0, 22.0, 60.0], [22.0, 60.0], 12.0, 0xFF55_FF55),
            ([20.0, 44.0, 28.0, 60.0], [28.0, 60.0], 8.0, 0xFF55_FF55),
            ([26.0, 44.0, 34.0, 60.0], [34.0, 60.0], 4.0, 0xFF55_FF55),
            ([14.0, 62.0, 22.0, 78.0], [22.0, 69.0], 0.0, 0xFFFF_FF55),
            ([22.0, 62.0, 30.0, 78.0], [30.0, 69.0], 0.0, 0xFFFF_FF55),
            ([30.0, 62.0, 38.0, 78.0], [38.0, 69.0], 0.0, 0xFFFF_FF55),
            ([38.0, 62.0, 46.0, 78.0], [43.0, 69.0], 0.0, 0xFFFF_FF55),
        ];
        assert_eq!(actual.vertices.len(), glyphs.len() * 6);
        for (quad, ([l, t, r, b], [visible_r, visible_b], u, color)) in
            actual.vertices.as_chunks::<6>().0.iter().zip(glyphs)
        {
            let corners = [[l, t], [r, t], [r, b], [l, b]];
            let (min_u, max_u) = ((u + 0.01) / 32.0, (u + 4.0 - 0.01) / 32.0);
            let (min_v, max_v) = (0.01 / 16.0, 7.99 / 16.0);
            let uvs = [
                [min_u, min_v],
                [max_u, min_v],
                [max_u, max_v],
                [min_u, max_v],
            ];
            for (v, corner) in quad.iter().zip([0, 1, 2, 0, 2, 3]) {
                assert_eq!(
                    (v.pos, v.uv, v.color, v.text),
                    (corners[corner], uvs[corner], color, 1)
                );
            }
            assert_eq!(
                [
                    quad[0].pos[0].max(clip[0]),
                    quad[0].pos[1].max(clip[1]),
                    quad[2].pos[0].min(clip[2]),
                    quad[2].pos[1].min(clip[3])
                ],
                [l, t, visible_r, visible_b],
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn local_tooltip_draws_deep_text_but_refuses_unrepresentable_metadata() {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let mut deep = mod_api::Text::literal("Tooltip");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                deep = mod_api::Text::literal("").append(deep);
            }
            let mut invalid = mod_api::Text::literal("Unrepresentable");
            invalid.metadata = Tag::compound([("test:nonfinite", Tag::Double(f64::NAN.to_bits()))]);

            let mut batch = crate::hud::HudBatch::scratch([400, 200]);
            let mut draw = GameDrawContext::new(game, &mut batch);
            assert_eq!(
                draw.gui_tooltip(&[deep.clone(), invalid], [16.0, 16.0]),
                Err(TextFormError::NonFinite)
            );
            assert!(draw.surface.vertices.is_empty());
            draw.gui_tooltip(&[deep], [16.0, 16.0]).unwrap();
            assert!(!draw.surface.vertices.is_empty());
        });
    }

    #[test]
    fn local_label_layout_handles_deep_trees_and_preserves_opaque_styles() {
        let mut opaque_dialog = Tag::ByteArray(vec![7, 8]);
        for _ in 0..600 {
            let mut map = std::collections::BTreeMap::new();
            map.insert("n".into(), opaque_dialog);
            opaque_dialog = Tag::Compound(map.into_iter().collect());
        }
        let mut style = mod_api::text::TextStyle::default();
        style.bold = Some(true);
        style.click = Some(mod_api::text::TextClick::ShowDialog(opaque_dialog.clone()));
        let mut deep = mod_api::Text::literal("AAAA BBBB").with_style(style);
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            let mut parent = mod_api::Text::literal("");
            parent.children.push(deep);
            deep = parent;
        }
        let lines = measured(&fake_font(6.0), &deep, 30.0, TextAlign::Left, 1.0).unwrap();
        assert_eq!(lines.lines.len(), 2);
        for line in &lines.lines {
            assert_eq!(line.text.style.bold, Some(true));
            assert_eq!(
                line.text.style.click,
                Some(mod_api::text::TextClick::ShowDialog(opaque_dialog.clone()))
            );
        }
    }

    #[test]
    fn engine_tooltip_font_measures_and_splits_deep_local_text() {
        let default_json = r#"{"providers":[{"type":"space","advances":{" ":2,"a":3}}]}"#;
        let alt_json = r#"{"providers":[{"type":"space","advances":{" ":4,"a":8}}]}"#;
        let source = sunrise_assets::resource_source::ResourceSource::indexed(BTreeMap::from([
            (
                "assets/minecraft/font/default.json".into(),
                sunrise_assets::resource_source::Resource::bytes(default_json.as_bytes().to_vec()),
            ),
            (
                "assets/minecraft/font/alt.json".into(),
                sunrise_assets::resource_source::Resource::bytes(alt_json.as_bytes().to_vec()),
            ),
        ]));
        let font = crate::font::Font::metrics(&source);
        let default_id = Identifier::parse("minecraft:default").unwrap();
        let alt_id = Identifier::parse("minecraft:alt").unwrap();

        let default_tf = EngineTooltipFont::new(&font, &default_id);
        let alt_tf = EngineTooltipFont::new(&font, &alt_id);
        assert_eq!(default_tf.id(), &default_id);
        assert_eq!(alt_tf.id(), &alt_id);
        assert_eq!(default_tf.line_height(), 9);

        let sample = mod_api::Text::literal("aa aa");
        assert_eq!(default_tf.width(&sample).unwrap(), 14);
        assert_eq!(alt_tf.width(&sample).unwrap(), 36);
        assert_eq!(default_tf.split(&sample, 16).unwrap().len(), 1);
        assert_eq!(alt_tf.split(&sample, 16).unwrap().len(), 2);

        let mut deep = mod_api::Text::literal("x");
        for _ in 0..600 {
            deep = mod_api::Text::literal("").append(deep);
        }
        assert_eq!(
            default_tf.width(&deep),
            default_tf.width(&mod_api::Text::literal("x"))
        );
        assert_eq!(default_tf.split(&deep, 100).unwrap().len(), 1);
    }
}
