#![cfg(test)]
#[cfg(test)]
pub(crate) mod render_tooltip_texture_tests {
    #[cfg(feature = "client")]
    use mod_api::ModEntry;
    #[cfg(feature = "client")]
    use mod_api::{ClientHost, Hooks, Identifier, RenderTooltipTextureEvent};
    #[cfg(feature = "client")]
    use mod_platform::testing::with_mods;
    #[cfg(feature = "client")]
    use sunrise_client::hud::{HudBatch, Rect};
    #[cfg(feature = "client")]
    use sunrise_items::items::ItemStack;

    /// Gives a stick's tooltip the `lobby:gold` style and a text tooltip a red background.
    #[cfg(feature = "client")]
    struct Gold;
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for Gold {
        fn render_tooltip_texture(
            &self,
            host: &dyn ClientHost,
            event: &mut RenderTooltipTextureEvent<'_>,
        ) {
            assert_eq!(*event.texture, None);
            assert_eq!(
                (event.background, event.border_top, event.border_bottom),
                (0xf0100010, 0x505000ff, 0x5028007f)
            );
            match event.stack {
                Some(stack) if stack.item().id(host) == Some("minecraft:stick") => {
                    *event.texture = Some(Identifier::parse("lobby:gold").unwrap());
                }
                Some(_) => {}
                None => event.background = 0xffff0000,
            }
        }
    }

    #[cfg(feature = "client")]
    fn sprite(u0: f32) -> Rect {
        Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.125,
            v1: 0.125,
            w: 100,
            h: 100,
        }
    }
    #[cfg(feature = "client")]
    fn drawn(
        game: &sunrise_client_gameplay::game::ClientGame<'_>,
        stack: Option<&ItemStack>,
    ) -> Vec<([f32; 2], u32)> {
        let mut batch = HudBatch::scratch([320, 240]);
        game.tooltip_box(&mut batch, stack, 40.0, 30.0, 60.0, 8.0);
        batch.vertices.iter().map(|v| (v.uv, v.color)).collect()
    }
    #[cfg(feature = "client")]
    fn in_sprite(uv: [f32; 2], r: Rect) -> bool {
        (r.u0..=r.u1).contains(&uv[0]) && (r.v0..=r.v1).contains(&uv[1])
    }

    /// Vanilla 26.3 `TooltipRenderUtil.extractTooltipBackground` witness recorded by `tools/test_tooltip_display_parity.py`: a stack's `minecraft:tooltip_style` draws exact sprites, bounds, and colours.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "run by tools/test_tooltip_display_parity.py"]
    fn an_items_tooltip_style_draws_its_own_sprites() {
        run_an_items_tooltip_style_draws_its_own_sprites();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_an_items_tooltip_style_draws_its_own_sprites() {
        #[derive(serde::Deserialize)]
        struct Witness {
            minecraft_version: String,
            cases: Vec<Case>,
        }
        #[derive(serde::Deserialize)]
        struct Case {
            label: String,
            item: String,
            tooltip_style: String,
            #[serde(rename = "box")]
            bounds: [i32; 4],
            draws: Vec<Draw>,
        }
        #[derive(serde::Deserialize)]
        struct Draw {
            sprite: String,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            color: u32,
        }

        let path =
            std::env::var_os("SUNRISE_TOOLTIP_STYLE_SPRITES_WITNESS").unwrap_or_else(|| {
                panic!(
                    "SUNRISE_TOOLTIP_STYLE_SPRITES_WITNESS is unset; run tools/test_tooltip_display_parity.py"
                )
            });
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "failed to read witness {} (record via tools/test_tooltip_display_parity.py): {e}",
                std::path::Path::new(&path).display()
            )
        });
        let witness: Witness = serde_json::from_str(&raw).unwrap();
        assert_eq!(witness.minecraft_version, "26.3");
        assert!(!witness.cases.is_empty());

        let vertex_bits = |batch: &HudBatch| -> Vec<([u32; 2], [u32; 2], u32)> {
            batch
                .vertices
                .iter()
                .map(|v| {
                    (
                        [v.pos[0].to_bits(), v.pos[1].to_bits()],
                        [v.uv[0].to_bits(), v.uv[1].to_bits()],
                        v.color,
                    )
                })
                .collect()
        };

        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            for case in &witness.cases {
                let [bx, by, bw, bh] = case.bounds.map(|v| v as f32);
                let mut plain_batch = HudBatch::scratch([320, 240]);
                game.tooltip_box(
                    &mut plain_batch,
                    Some(&ItemStack::new(&case.item, 1)),
                    bx,
                    by,
                    bw,
                    bh,
                );
                let plain = vertex_bits(&plain_batch);

                let mut styled = ItemStack::new(&case.item, 1);
                let given = sunrise_items::item_data::command_components(&format!(
                    "[tooltip_style='{}']",
                    case.tooltip_style
                ))
                .unwrap();
                (styled.damage, styled.extra) =
                    sunrise_items::item_data::decode_components(Some(&given));

                let mut inserted = Vec::with_capacity(case.draws.len());
                for (index, draw) in case.draws.iter().enumerate() {
                    let key = draw
                        .sprite
                        .strip_prefix("minecraft:")
                        .unwrap_or(&draw.sprite)
                        .to_owned();
                    let rect = sprite(index as f32 * 0.25);
                    game.client.gui.atlas.sprites.insert(key.clone(), rect);
                    inserted.push((key, rect));
                }

                let mut batch = HudBatch::scratch([320, 240]);
                game.tooltip_box(&mut batch, Some(&styled), bx, by, bw, bh);

                let actual = vertex_bits(&batch);
                assert_ne!(actual, plain, "{}", case.label);

                let mut expected = Vec::with_capacity(case.draws.len() * 6);
                for (draw, (_, rect)) in case.draws.iter().zip(&inserted) {
                    let (x, y, w, h) = (draw.x as f32, draw.y as f32, draw.w as f32, draw.h as f32);
                    let corners = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
                    let uvs = [
                        [rect.u0, rect.v0],
                        [rect.u1, rect.v0],
                        [rect.u1, rect.v1],
                        [rect.u0, rect.v1],
                    ];
                    for corner in [0, 1, 2, 0, 2, 3] {
                        expected.push((
                            [corners[corner][0].to_bits(), corners[corner][1].to_bits()],
                            [uvs[corner][0].to_bits(), uvs[corner][1].to_bits()],
                            draw.color,
                        ));
                    }
                }
                assert_eq!(actual, expected, "{}", case.label);

                for (key, _) in inserted {
                    game.client.gui.atlas.sprites.remove(&key);
                }
            }
        });
    }

    /// `RenderTooltipEvent.Texture`: a mod's tooltip style draws its background and frame
    /// sprites from the GUI atlas, follows the atlas a resource reload restitches, and falls
    /// back to the fills when no pack has it; its colours change the fills; and an unheard
    /// tooltip draws as before.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_mod_gives_a_tooltip_its_own_texture_or_colours() {
        let stick = ItemStack::new("minecraft:stick", 1);
        let stone = ItemStack::new("minecraft:stone", 1);
        let mods: mod_api::Mods = vec![ModEntry::new("gold", Box::new(Gold))].into();
        // Keep baseline and modded draws on one atlas so `atlas.white`'s UV matches (#4437).
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let unheard = [None, Some(&stick)].map(|stack| drawn(game, stack));
            // The solid fills: the background's three, the two borders' and their gradient sides.
            assert_eq!(unheard[0].len(), 7 * 6);
            assert_eq!(unheard[0], unheard[1]);
            with_mods(mods, || {
                let names =
                    ["background", "frame"].map(|part| format!("lobby:tooltip/gold_{part}"));
                // No pack has the style: the stick keeps the fills.
                assert_eq!(drawn(game, Some(&stick)), unheard[1]);
                for (first, then) in [(0.0, 0.5), (0.25, 0.75)] {
                    let rects = [sprite(first), sprite(then)];
                    for (name, rect) in names.iter().zip(rects) {
                        game.client.gui.atlas.sprites.insert(name.clone(), rect);
                    }
                    let quads = drawn(game, Some(&stick));
                    assert_ne!(quads, unheard[1]);
                    assert!(
                        quads.iter().all(|&(uv, color)| color == u32::MAX
                            && rects.iter().any(|&r| in_sprite(uv, r)))
                    );
                    for rect in rects {
                        assert!(quads.iter().any(|&(uv, _)| in_sprite(uv, rect)));
                    }
                }
                // Another item's tooltip is unchanged; the text tooltip's background is red.
                assert_eq!(drawn(game, Some(&stone)), unheard[1]);
                let red = drawn(game, None);
                assert_eq!(red.len(), unheard[0].len());
                assert_ne!(red, unheard[0]);
            });
        });
    }
}
