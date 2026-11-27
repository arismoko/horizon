#![cfg(feature = "client")]
#![cfg(test)]
//! A client mod's dynamic textures (`ClientRenderClientHost::create_dynamic_texture`): one GPU
//! texture each, drawn by `GuiCanvas::texture` on the real HUD pipeline, changed by region
//! writes applied in order, and released with the call that made them or with the mod.
use mod_api::{DynamicTextureError, GuiBlitRegion, Hooks, Identifier, ModEntry};
use mod_platform::testing::with_mods;
use sunrise_client::{gfx::entity_pixel_test::render_hud_frames, hud::HudBatch};
use sunrise_client_gameplay::game::ClientGame;
use sunrise_level::crash::{ModCall, enter_native_mod};

const SIZE: [u32; 2] = [128, 128];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A mod with no hook and no mixin: it only uses the API.
struct ApiOnly;

#[mod_api::__subscriptions]
impl Hooks for ApiOnly {}

/// `f` run as mod `id`'s client tick.
fn as_mod<R>(id: &'static str, f: impl FnOnce() -> R) -> R {
    let _call = enter_native_mod(ModCall {
        id,
        hook: "client_tick_post",
        part: None,
    });
    f()
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    pixels[((y * SIZE[0] + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

/// A hidden offscreen window and renderer on `game`, so the dynamic textures have a device.
fn attach_renderer(game: &mut ClientGame<'_>) -> winit::event_loop::EventLoop<()> {
    let (event_loop, window) =
        sunrise_client_gameplay::game::native_entity_refs::tests::test_window([640, 480]);
    game.options.offscreen = true;
    game.client.window.0 = Some(window.clone());
    {
        let renderer = game.new_renderer(window).unwrap();
        game.set_renderer(renderer);
    }
    event_loop
}

/// A batch of the frame's dynamic texture commands and `texture` drawn into `dst`, its whole
/// `tex`-sized image.
fn frame(game: &mut ClientGame<'_>, draws: &[(&Identifier, [f32; 4], [i32; 2])]) -> HudBatch {
    let mut batch = HudBatch::scratch(SIZE);
    game.dynamic_texture_commands(&mut batch.dynamic_commands);
    game.draw_gui_canvas(&mut batch, None, &mut |canvas| {
        for (id, dst, tex) in draws {
            canvas.texture(
                id,
                *dst,
                GuiBlitRegion {
                    uv: [0.0, 0.0],
                    src: *tex,
                    tex: *tex,
                },
                u32::MAX,
            )?;
        }
        Ok(())
    })
    .unwrap();
    batch
}

/// A texture the mod made draws under its id on the real HUD pipeline, a region write changes
/// only the pixels it names in the next frame, and a release ends the draw.
#[test]
#[ignore = "daily"]
fn a_dynamic_texture_draws_through_the_gui_texture_path_and_takes_a_region_update() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let _event_loop = attach_renderer(game);
            let id = Identifier::parse("api-only:dynamic/panel").unwrap();
            as_mod("api-only", || {
                game.create_dynamic_texture(&id, 4, 4, 1, &GREEN.repeat(16))
            })
            .unwrap();
            assert_eq!(
                as_mod("other", || game.release_dynamic_texture(&id)),
                Err(DynamicTextureError::NotOwned)
            );
            let dst = [16.0, 16.0, 32.0, 32.0];
            let first = frame(game, &[(&id, dst, [4, 4])]);
            as_mod("api-only", || {
                game.update_dynamic_texture_region(&id, 0, [1, 1], [2, 2], &RED.repeat(4))
            })
            .unwrap();
            let second = frame(game, &[(&id, dst, [4, 4])]);
            let frames = render_hud_frames(&[&first, &second], &game.client.gui.atlas, SIZE)
                .expect("software Vulkan draw");
            // Texel (x, y) of the 4 x 4 image covers 8 x 8 pixels from (16, 16).
            let texel = |x: u32, y: u32| (16 + x * 8 + 4, 16 + y * 8 + 4);
            for (frame, red) in [(&frames[0], false), (&frames[1], true)] {
                for (x, y) in [(0, 0), (3, 3), (0, 3), (3, 0)] {
                    let (px, py) = texel(x, y);
                    assert_eq!(pixel(frame, px, py), GREEN, "texel {x},{y} is untouched");
                }
                for (x, y) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
                    let (px, py) = texel(x, y);
                    assert_eq!(
                        pixel(frame, px, py),
                        if red { RED } else { GREEN },
                        "texel {x},{y}"
                    );
                }
                assert_eq!(pixel(frame, 4, 4), BLACK, "outside the texture");
            }
            as_mod("api-only", || game.release_dynamic_texture(&id)).unwrap();
            assert_eq!(
                as_mod("api-only", || game.release_dynamic_texture(&id)),
                Err(DynamicTextureError::Unknown)
            );
        });
    });
}

/// Any mod may draw any live id (the draw has no owner check), and an id released after its
/// draw was submitted draws vanilla's missing texture, as `TextureManager.release` leaves it.
#[test]
#[ignore = "daily"]
fn a_texture_released_after_its_draw_was_submitted_draws_the_missing_texture() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let _event_loop = attach_renderer(game);
            let id = Identifier::parse("api-only:dynamic/panel").unwrap();
            as_mod("api-only", || {
                game.create_dynamic_texture(&id, 4, 4, 1, &GREEN.repeat(16))
            })
            .unwrap();
            let created = frame(game, &[]);
            // Submitted while live, as another mod may, then released before the frame draws.
            let mut submitted = frame(game, &[(&id, [16.0, 16.0, 32.0, 32.0], [4, 4])]);
            as_mod("api-only", || game.release_dynamic_texture(&id)).unwrap();
            game.dynamic_texture_commands(&mut submitted.dynamic_commands);
            let frames = render_hud_frames(&[&created, &submitted], &game.client.gui.atlas, SIZE)
                .expect("software Vulkan draw");
            // `generateMissingImage`: black top left, magenta top right.
            assert_eq!(pixel(&frames[1], 20, 20), BLACK);
            assert_eq!(pixel(&frames[1], 44, 20), [248, 0, 248, 255]);
        });
    });
}

/// A foreign id is `NotOwned`, even where the resource stack has the id (vanilla's textures):
/// the owner check comes first.
#[test]
#[ignore = "daily"]
fn a_foreign_id_is_not_owned_before_the_resource_stack_is_asked() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let _event_loop = attach_renderer(game);
            let green = GREEN.repeat(16 * 16);
            let make = |game: &mut ClientGame<'_>, id: &str| {
                let id = Identifier::parse(id).unwrap();
                as_mod("api-only", || {
                    game.create_dynamic_texture(&id, 16, 16, 1, &green)
                })
            };
            assert_eq!(
                make(game, "minecraft:textures/gui/icons.png"),
                Err(DynamicTextureError::NotOwned)
            );
            assert_eq!(make(game, "api-only:dynamic/free"), Ok(()));
        });
    });
}

/// A create the device refuses (a side one texel past its limit, which a table bound to a larger
/// `max_side` lets through) leaves the id in the client table with no GPU texture: it draws
/// vanilla's missing texture, and a later update of another size creates it again.
#[test]
#[ignore = "daily"]
fn a_create_the_device_refuses_draws_the_missing_texture() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let _event_loop = attach_renderer(game);
            let renderer = game.client.renderer.0.as_ref().unwrap();
            let (store, limit) = (renderer.hud.dynamic().id(), renderer.max_texture_side());
            let id = Identifier::parse("api-only:dynamic/too-wide").unwrap();
            let wide = limit + 1;
            // The table believes the device is larger than it is; the frame's own bind
            // reads the real limit again, after this create is queued.
            game.client.dynamic_textures.bind(store, wide);
            game.client
                .dynamic_textures
                .create("api-only", &id, [wide, 1], 1, &GREEN.repeat(wide as usize))
                .unwrap();
            let refused = frame(game, &[(&id, [16.0, 16.0, 32.0, 32.0], [wide as i32, 1])]);
            let frames = render_hud_frames(&[&refused], &game.client.gui.atlas, SIZE)
                .expect("software Vulkan draw");
            // `generateMissingImage`: black top left, magenta top right.
            assert_eq!(pixel(&frames[0], 20, 20), BLACK);
            assert_eq!(pixel(&frames[0], 44, 20), [248, 0, 248, 255]);
            // A different size creates it again, and the texture then draws.
            as_mod("api-only", || {
                game.update_dynamic_texture(&id, 4, 4, &GREEN.repeat(16))
            })
            .unwrap();
            let made = frame(game, &[(&id, [16.0, 16.0, 32.0, 32.0], [4, 4])]);
            let frames = render_hud_frames(&[&made], &game.client.gui.atlas, SIZE)
                .expect("software Vulkan draw");
            assert_eq!(pixel(&frames[0], 20, 20), GREEN);
        });
    });
}
