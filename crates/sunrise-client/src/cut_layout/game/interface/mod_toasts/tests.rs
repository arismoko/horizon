use mod_api::{AddToastError, Identifier, ModToast, ToastIcon, ToastKind};
use mod_api::{ClientHost, Flow, Hooks, ModEntry, ScreenRenderPostEvent, Text, ToastAddEvent};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sunrise_client::hud::rgba;
use sunrise_client_gameplay::game::client_side::mod_toasts::Entry;
use sunrise_client_gameplay::game::toast_manager::Toast;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::in_own_process,
};

const OPTIONS_MARKER: u32 = 0xFF5D_3E5D;

#[test]
#[ignore = "daily"]
fn client_toasts_share_the_draw_list_and_admit_replacements_before_updating() {
    in_own_process(
        "cut_layout::game::client_side::mod_toasts::tests::client_toasts_share_the_draw_list_and_admit_replacements_before_updating",
        || {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new(
                "toast_fixture",
                Box::new(Filter(seen.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let saves = tempfile::tempdir().unwrap();
                    game.show_title(game.new_title(saves.path().to_owned()));
                    game.open_options(true);
                    let kind = Identifier::parse("toast_fixture:progress").unwrap();
                    let sprite_id = Identifier::parse("minecraft:icon/checkmark").unwrap();
                    let sprite = *game.client.gui.atlas.gui_rect("icon/checkmark").unwrap();
                    let apple = *game.client.gui.atlas.texture_rect("item/apple").unwrap();
                    let mut first = ModToast::new(Text::literal("Queued sprite"));
                    first.kind = Some(kind.clone());
                    first.icon = Some(ToastIcon::Sprite(sprite_id));
                    let mut description =
                        Text::literal(format!("First line\n{}\nLast line", "M".repeat(42)));
                    description.style.color = Some("#234567".into());
                    first.description = Some(description);

                    let host: &dyn ClientHost = game;
                    let apple_key = host
                        .item_key(&Identifier::parse("minecraft:apple").unwrap())
                        .unwrap();
                    let apple_stack = mod_api::ItemStack::new(apple_key, 1).unwrap();
                    let mut second = ModToast::new(Text::literal("Queued item"));
                    second.icon = Some(ToastIcon::Item(apple_stack.clone()));
                    host.add_toast(first).unwrap();
                    host.add_toast(second).unwrap();

                    let mut bad_stack = apple_stack;
                    bad_stack
                        .set_components(
                            [("minecraft:consumable".into(), mod_api::Tag::Int(1))]
                                .into_iter()
                                .collect(),
                        )
                        .unwrap();
                    let mut refused = ModToast::new(Text::literal("Invalid icon"));
                    refused.icon = Some(ToastIcon::Item(bad_stack));
                    assert_eq!(
                        host.add_toast(refused),
                        Err(AddToastError::Icon(
                            mod_api::StackWriteError::InvalidEffectPayload
                        ))
                    );
                    assert!(seen.lock().unwrap().is_empty(), "no synchronous dispatch");

                    let draw = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                        game.hud([960, 540])
                    };
                    draw(game);
                    assert_eq!(*seen.lock().unwrap(), ["Queued sprite", "Queued item"]);
                    assert_eq!(
                        mod_entry(&game.client.toast_manager.visible[0].toast)
                            .request
                            .toast
                            .display_time(),
                        5000.0
                    );
                    draw(game);
                    assert_eq!(
                        *seen.lock().unwrap(),
                        ["Queued sprite", "Queued item", "Follow-up"]
                    );
                    for entry in &mut game.client.toast_manager.visible {
                        let Toast::Mod(entry) = &mut entry.toast else {
                            panic!("expected mod toast")
                        };
                        entry.request.toast.slide.origin =
                            Some(Instant::now() - Duration::from_millis(700));
                    }
                    let frame = draw(game);
                    let options_end = frame
                        .vertices
                        .iter()
                        .rposition(|v| v.color == OPTIONS_MARKER)
                        .expect("the title Options path drew its screen post hook");
                    let after_options = &frame.vertices[options_end + 1..];
                    assert!(
                        after_options.iter().any(|v| v.text != 0
                            && v.color == rgba(0x12, 0x34, 0x56, 255)
                            && v.pos[0] < frame.width),
                        "edited toast title must draw after Options"
                    );
                    assert!(
                        frame.vertices.iter().any(|v| v.text != 0
                            && v.color == rgba(0x23, 0x45, 0x67, 255)
                            && v.pos[1] >= 31.0
                            && v.pos[0] < frame.width),
                        "long description must wrap below the first description line"
                    );
                    assert_eq!(
                        game.client.toast_manager.visible[1].slot, 3,
                        "the first toast's three slots keep the next toast below it"
                    );
                    assert!(
                        frame.vertices.iter().any(|v| v.uv == [sprite.u0, sprite.v0]
                            && v.pos[1] == 8.0
                            && v.pos[0] < frame.width),
                        "GUI sprite must draw at the toast's icon position"
                    );
                    assert!(
                        frame
                            .vertices
                            .iter()
                            .any(|v| (apple.u0..=apple.u1).contains(&v.uv[0])
                                && (apple.v0..=apple.v1).contains(&v.uv[1])
                                && v.pos[1] >= 104.0
                                && v.pos[0] < frame.width),
                        "item icon must draw in the next toast"
                    );

                    game.toggle_hide_gui();
                    assert!(
                        draw(game)
                            .vertices
                            .iter()
                            .all(|v| v.color != rgba(0x12, 0x34, 0x56, 255)),
                        "F1 hides the toast, not the Options screen"
                    );
                    game.toggle_hide_gui();

                    let mut cancelled = ModToast::new(Text::literal("Ignore replacement"));
                    cancelled.kind = Some(kind.clone());
                    (&*game as &dyn ClientHost).add_toast(cancelled).unwrap();
                    draw(game);
                    let Toast::Mod(shown) = &mut game.client.toast_manager.visible[0].toast else {
                        panic!("expected mod toast")
                    };
                    assert_eq!(shown.request.toast.title.plain(), "Edited sprite");
                    assert_eq!(seen.lock().unwrap().last().unwrap(), "Ignore replacement");

                    let old_origin = Instant::now() - Duration::from_secs(60);
                    shown.request.toast.slide.origin = Some(old_origin);
                    let mut replacement = ModToast::new(Text::literal("New content"));
                    replacement.kind = Some(kind);
                    replacement.display_ms = Some(2500);
                    (&*game as &dyn ClientHost).add_toast(replacement).unwrap();
                    draw(game);
                    let shown = mod_entry(&game.client.toast_manager.visible[0].toast);
                    assert_eq!(game.client.toast_manager.visible.len(), 3);
                    assert!(game.client.toast_manager.queued.is_empty());
                    assert_eq!(shown.request.toast.title.plain(), "New content");
                    assert_eq!(game.client.toast_manager.visible[0].slot, 0);
                    assert_eq!(shown.request.toast.slide.origin, Some(old_origin));
                    assert_eq!(shown.request.toast.slide.portion, 1.0);
                    assert_eq!(shown.request.toast.display_time(), 2500.0);

                    game.add_toast(ModToast::new(Text::literal("Queued at disconnect")))
                        .unwrap();
                    draw(game);
                    assert_eq!(game.client.toast_manager.queued.len(), 1);
                    game.add_toast(ModToast::new(Text::literal("Pending at disconnect")))
                        .unwrap();
                    assert_eq!(game.client.toast_manager.pending_mod.borrow().len(), 1);
                    game.leave_server(None).unwrap();
                    draw(game);
                    assert!(game.client.toast_manager.visible.is_empty());
                    assert!(game.client.toast_manager.queued.is_empty());
                    assert_eq!(seen.lock().unwrap().last().unwrap(), "Queued at disconnect");

                    game.add_toast(ModToast::new(Text::literal("§a"))).unwrap();
                    draw(game);
                    draw(game);
                    let Toast::Mod(entry) = &mut game.client.toast_manager.visible[0].toast else {
                        panic!("expected formatting-only toast");
                    };
                    entry.request.toast.slide.origin =
                        Some(Instant::now() - Duration::from_secs(1));
                    let rect = *game.client.gui.atlas.gui_rect("toast/system").unwrap();
                    let frame = draw(game);
                    let frame_bounds = frame
                        .vertices
                        .iter()
                        .filter(|v| {
                            v.text == 0
                                && (rect.u0..=rect.u1).contains(&v.uv[0])
                                && (rect.v0..=rect.v1).contains(&v.uv[1])
                        })
                        .fold(
                            [
                                f32::INFINITY,
                                f32::INFINITY,
                                f32::NEG_INFINITY,
                                f32::NEG_INFINITY,
                            ],
                            |[x0, y0, x1, y1], vertex| {
                                let [x, y] = vertex.pos;
                                [x0.min(x), y0.min(y), x1.max(x), y1.max(y)]
                            },
                        );
                    assert_eq!(frame_bounds, [frame.width - 190.0, 0.0, frame.width, 32.0]);
                });
            });
        },
    );
}

fn mod_entry(toast: &Toast) -> &Entry {
    let Toast::Mod(entry) = toast else {
        panic!("expected mod toast")
    };
    entry
}

struct Filter(Arc<Mutex<Vec<String>>>);

#[mod_api::__subscriptions]
impl Hooks for Filter {
    fn screen_render_post(&self, _: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if event.screen.id == "options" {
            event.draw.fill([1.0, 1.0, 2.0, 2.0], OPTIONS_MARKER);
        }
    }

    fn toast_add(&self, host: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
        if event.kind != ToastKind::Mod {
            return Flow::Continue;
        }
        let mod_api::text::TextContent::Literal(title) = &event.title.content else {
            return Flow::Continue;
        };
        self.0.lock().unwrap().push(title.clone());
        match title.as_str() {
            "Ignore replacement" => Flow::Cancel,
            "Queued sprite" => {
                host.add_toast(ModToast::new(Text::literal("Follow-up")))
                    .unwrap();
                *event.title = Text::literal("Edited sprite");
                event.title.style.color = Some("#123456".into());
                Flow::Continue
            }
            _ => Flow::Continue,
        }
    }
}
