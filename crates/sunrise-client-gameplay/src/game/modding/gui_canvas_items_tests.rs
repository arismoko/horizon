#![cfg(test)]
//! Seeded item models and decorations through a mod's GUI canvas and real dispatch.
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use mod_api::{
    ClientHost, ComponentHoverEvent, GuiCanvas, GuiDrawError, Hooks, ItemBarEvent,
    ItemDecorationsEvent, ItemDisplayContext, ItemModelLayer, ItemModelUpdateInput, ItemPart,
    ModEntry, RegisterItemModelsEvent, RegistrationContext, client::hud_layout::PoseOp,
};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};
use sunrise_assets::item_model::ItemModelNode;
use sunrise_client::hud::HudBatch;
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;

const MODEL: &str = "painter:icon";
const ITEM: &str = "minecraft:paper";
const BAR: u32 = 0xFF00_FF00;
type Answers = Arc<Mutex<Vec<Result<(), GuiDrawError>>>>;
type Resolves = Arc<Mutex<Vec<(i32, u32, ItemDisplayContext)>>>;

struct Painter {
    answers: Answers,
    resolves: Resolves,
    bar_width: i32,
}

#[mod_api::__subscriptions]
impl Hooks for Painter {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
        let key = r.item_key(ITEM);
        r.item_part(key, Decorator(self.bar_width));
    }

    fn register_item_models(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterItemModelsEvent<'_>,
    ) {
        let resolves = self.resolves.clone();
        event.register(mod_api::Identifier::parse(MODEL).unwrap(), move |_, _| {
            let resolves = resolves.clone();
            Ok(
                move |_: &dyn ClientHost,
                      input: &ItemModelUpdateInput<'_>,
                      layers: &mut Vec<ItemModelLayer>| {
                    resolves.lock().unwrap().push((
                        input.context.seed,
                        input.stack.count(),
                        input.context.display,
                    ));
                    layers.push(ItemModelLayer::new("paper"));
                },
            )
        });
    }

    fn component_hover(
        &self,
        host: &dyn ClientHost,
        event: &mut ComponentHoverEvent<'_>,
    ) -> mod_api::Flow {
        let stack = host
            .local_player()
            .and_then(|player| player.with_inventory(host, |inv| inv.get(0)?.project()))
            .flatten();
        if let Some(stack) = stack {
            let refuse = event.x == 50;
            let answer = event.draw.draw_gui(&mut |canvas| {
                canvas.scope(&[PoseOp::Translate(20.0, 30.0)], None, &mut |parent| {
                    parent.scope(
                        &[
                            PoseOp::Translate(8.0, 12.0),
                            PoseOp::Scale(0.5, 1.5),
                            PoseOp::Translate(-8.0, -12.0),
                        ],
                        None,
                        &mut |model: &mut dyn GuiCanvas| model.item(&stack, [0.0, 0.0], 37),
                    )?;
                    parent.item_decorations(&stack, [0.0, 0.0], None)
                })?;
                if refuse {
                    canvas.item(&stack, [f32::NAN, 0.0], 99)?;
                }
                Ok(())
            });
            self.answers.lock().unwrap().push(answer);
        }
        mod_api::Flow::Cancel
    }
}

#[derive(Clone)]
struct Decorator(i32);
#[mod_api::sunrise_part]
impl ItemPart for Decorator {
    fn item_bar(&self, _: &dyn ClientHost, event: &mut ItemBarEvent<'_>) {
        *event.bar = Some((self.0, BAR));
    }

    fn item_decorations(&self, _: &dyn ClientHost, event: &mut ItemDecorationsEvent<'_>) {
        if event.stack.count() == 1 {
            let at = [event.x + 20.0, event.y];
            event
                .draw
                .scope(&[], None, &mut |child| {
                    child.item_decorations(event.stack, at, None)
                })
                .unwrap();
        }
        event
            .draw
            .fill([event.x - 2.0, event.y + 2.0, 1.0, 2.0], 0xFFFF_0000)
            .unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn item_decorator_can_delegate_to_the_base_painter_without_recursion() {
    const NAME: &str = "game::client_side::gui_canvas_items_tests::item_decorator_can_delegate_to_the_base_painter_without_recursion";
    if run_fresh_cases(NAME, "SUNRISE_GUI_DECORATION_DELEGATION", &["run"]) {
        return;
    }
    let batch = draw_decorated_paper(5);
    let bars: Vec<_> = batch
        .vertices
        .as_chunks::<6>()
        .0
        .iter()
        .filter(|quad| quad[0].color == BAR)
        .map(|quad| quad[0].pos)
        .collect();
    assert_eq!(bars, [[10.0, 29.0], [30.0, 29.0]]);
    assert_eq!(
        batch
            .vertices
            .iter()
            .filter(|v| v.color == 0xFF00_00FF)
            .count(),
        6
    );
}

#[test]
#[ignore = "daily"]
fn canvas_wraps_extreme_item_bar_endpoints_like_java() {
    const NAME: &str = "game::client_side::gui_canvas_items_tests::canvas_wraps_extreme_item_bar_endpoints_like_java";
    if run_fresh_cases(NAME, "SUNRISE_GUI_EXTREME_BAR", &["run"]) {
        return;
    }
    let batch = draw_decorated_paper(i32::MAX);
    // In the slot-local pose, Java int addition wraps (2 + MAX) to -2147483647.
    // Converting that endpoint to f32 and translating by eight rounds to -2147483648.
    assert_eq!(
        batch.vertices[6..12]
            .iter()
            .map(|v| (v.pos, v.color))
            .collect::<Vec<_>>(),
        [
            ([-2_147_483_648.0, 29.0], BAR),
            ([10.0, 29.0], BAR),
            ([10.0, 30.0], BAR),
            ([-2_147_483_648.0, 29.0], BAR),
            ([10.0, 30.0], BAR),
            ([-2_147_483_648.0, 30.0], BAR),
        ]
    );
}

fn draw_decorated_paper(bar_width: i32) -> HudBatch {
    let mods: mod_api::Mods = vec![ModEntry::new(
        "painter",
        Box::new(Painter {
            answers: Answers::default(),
            resolves: Resolves::default(),
            bar_width,
        }),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let mut batch = HudBatch::scratch([320, 240]);
    with_mods(mods, || {
        with_client_game(|game| {
            let stack =
                mod_api::ItemStack::new(mod_api::__private::item_key(ITEM).unwrap(), 1).unwrap();
            game.draw_gui_canvas(&mut batch, None, &mut |canvas| {
                canvas.item_decorations(&stack, [8.0, 16.0], None)
            })
            .unwrap();
        });
    });
    batch
}

#[test]
#[ignore = "daily"]
fn scoped_items_keep_the_seed_and_draw_decorations_outside_the_squeeze() {
    const NAME: &str = "game::client_side::gui_canvas_items_tests::scoped_items_keep_the_seed_and_draw_decorations_outside_the_squeeze";
    if run_fresh_cases(NAME, "SUNRISE_GUI_SCOPED_ITEMS", &["run"]) {
        return;
    }
    use crate::game::native_client_session::tests::logged_client;

    use mod_api::__private::game::RegistrationContextNew;
    use sunrise_client::item_render::{bind_item_model_factories, decode_item_model};

    let answers = Answers::default();
    let resolves = Resolves::default();
    let mods: mod_api::Mods = vec![ModEntry::new(
        "painter",
        Box::new(Painter {
            answers: answers.clone(),
            resolves: resolves.clone(),
            bar_width: 5,
        }),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    with_mods(mods, || {
        let mut types = Vec::new();
        sunrise_gameplay_common::game::native_hooks::register_item_models::native_register_item_models(
            &RegistrationContext::new(),
            &mut types,
        );
        let (factories, kept) = bind_item_model_factories(types);
        assert_eq!(kept, [MODEL]);
        let bound = decode_item_model(
            Some(&factories),
            MODEL,
            &Default::default(),
            &ItemModelNode::decode,
        )
        .unwrap()
        .unwrap()
        .model;
        with_client_game(|game| {
            let _server = logged_client(game);
            game.client.gui.atlas.font = crate::game::host::draw::canvas_test_font();
            game.clock.reset();
            game.cooldowns.add(Arc::from(ITEM), 4);
            assert!(game.cooldowns.tick().is_empty());
            assert!(game.cooldowns.tick().is_empty());
            let renderer = Arc::get_mut(&mut game.client.client_pack.item_renderer)
                .expect("the game's item renderer is shared");
            let definition = Arc::make_mut(&mut renderer.models)
                .definitions
                .get_mut(ITEM)
                .unwrap();
            let paper = std::mem::replace(&mut definition.model, ItemModelNode::Empty);
            definition.model = ItemModelNode::Registered {
                model: bound,
                transformation: None,
                models: vec![("paper".into(), paper)],
            };
            // This count is legal to draw, but exceeds paper's resident stack limit.
            game.inventory[0] = Some(ItemStack::new(ITEM, 65));
            let hover = Text::from(serde_json::json!({
                "text": "x", "hover_event": { "action": "show_text", "value": "hi" }
            }));
            let style = &hover.spans[0].style;
            let mut canvas = HudBatch::scratch([320, 240]);
            game.component_hover_effect(&mut canvas, style, (40.0, 30.0));
            let committed = bytemuck::cast_slice::<_, u8>(&canvas.vertices).to_vec();
            game.component_hover_effect(&mut canvas, style, (50.0, 30.0));
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&canvas.vertices),
                committed.as_slice()
            );

            // Paper's flat GUI model covers its 16-pixel square. Its bounds identify the
            // model independently of face order. The bar, text and marker use the parent pose.
            let bar = canvas
                .vertices
                .iter()
                .position(|v| v.color == BAR)
                .expect("no bar")
                - 6;
            let mut min = [f32::INFINITY; 2];
            let mut max = [f32::NEG_INFINITY; 2];
            for v in &canvas.vertices[..bar] {
                for axis in 0..2 {
                    min[axis] = min[axis].min(v.pos[axis]);
                    max[axis] = max[axis].max(v.pos[axis]);
                }
            }
            assert_eq!([min, max], [[24.0, 24.0], [32.0, 48.0]], "model bounds");

            // Vanilla slot offsets: the bar starts at +2,+13, a half cooldown covers
            // the bottom eight pixels, and count "65" advances 3 + 4 pixels. Text
            // emits each glyph's +1,+1 shadow before that glyph's foreground.
            let expected = [
                ([22.0, 43.0, 35.0, 45.0], 0xFF00_0000, None),
                ([22.0, 43.0, 27.0, 44.0], BAR, None),
                ([20.0, 38.0, 36.0, 46.0], 0x7FFF_FFFF, None),
                ([31.0, 40.0, 35.0, 48.0], 0xFF3F_3F3F, Some(16.0)),
                ([30.0, 39.0, 34.0, 47.0], 0xFFFF_FFFF, Some(16.0)),
                ([34.0, 40.0, 38.0, 48.0], 0xFF3F_3F3F, Some(20.0)),
                ([33.0, 39.0, 37.0, 47.0], 0xFFFF_FFFF, Some(20.0)),
                ([18.0, 32.0, 19.0, 34.0], 0xFF00_00FF, None),
            ];
            assert_eq!(canvas.vertices.len() - bar, expected.len() * 6);
            for (quad, ([l, t, r, b], color, u)) in canvas.vertices[bar..]
                .as_chunks::<6>()
                .0
                .iter()
                .zip(expected)
            {
                let corners = [[l, t], [r, t], [r, b], [l, b]];
                for (v, corner) in quad.iter().zip([0, 1, 2, 0, 2, 3]) {
                    assert_eq!(
                        (v.pos, v.color, v.text),
                        (corners[corner], color, u32::from(u.is_some()))
                    );
                }
                if let Some(u) = u {
                    assert_eq!(quad[0].uv, [(u + 0.01) / 32.0, 0.01 / 16.0]);
                    assert_eq!(quad[2].uv, [(u + 4.0 - 0.01) / 32.0, 7.99 / 16.0]);
                }
            }

            game.inventory[0] = None;
            game.component_hover_effect(&mut canvas, style, (40.0, 30.0));
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&canvas.vertices),
                committed.as_slice()
            );

            // The ordinary hotbar resolves real sparse slots, then the off hand.
            game.inventory[..9].fill(None);
            game.inventory[2] = Some(ItemStack::new(ITEM, 2));
            game.inventory[7] = Some(ItemStack::new(ITEM, 7));
            game.offhand = Some(ItemStack::new(ITEM, 10));
            let mut hotbar = HudBatch::scratch([320, 240]);
            let (inputs, source) = game
                .camera_hotbar(crate::game::remote_camera::CameraPlayer::Own)
                .expect("local player hotbar inputs");
            game.draw_item_hotbar(&mut hotbar, [320, 240], &inputs, source);
        });
    });
    assert_eq!(
        *answers.lock().unwrap(),
        [Ok(()), Err(GuiDrawError::NotFinite)]
    );
    assert_eq!(
        *resolves.lock().unwrap(),
        [(37, 65), (37, 65), (3, 2), (8, 7), (10, 10)].map(|(seed, count)| (
            seed,
            count,
            ItemDisplayContext::Gui
        ))
    );
}
