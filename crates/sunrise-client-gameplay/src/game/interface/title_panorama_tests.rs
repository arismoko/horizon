use super::lightmap_expression_tests::{corner_of, frame};
use super::native_entity_refs::tests::{run_fresh_cases, test_window, with_client_game};
use super::resource_tables_client::UiResources;
use sunrise_assets::resource_source::Resource;

fn red_face() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[255, 0, 0, 255].repeat(4))
        .unwrap();
    bytes
}

#[test]
#[ignore = "daily"]
fn the_title_frame_draws_the_loaded_panorama() {
    if run_fresh_cases(
        "game::client_side::title_panorama_tests::the_title_frame_draws_the_loaded_panorama",
        "SUNRISE_TITLE_PANORAMA_CASE",
        &["draw"],
    ) {
        return;
    }
    let _gpu = sunrise_client_render::gfx::lock_test_gpu();
    with_client_game(|game| {
        let face = red_face();
        let source = game.common.pack.source.extended(
            (0..6)
                .map(|i| {
                    (
                        format!("assets/minecraft/textures/gui/title/background/panorama_{i}.png"),
                        Resource::bytes(face.clone()),
                    )
                })
                .collect(),
        );
        let root = &game.common.options.resources;
        game.client.gui = UiResources::load(&source, root).unwrap();
        let title = game.new_title(root.join("saves"));
        game.common.options.offscreen = true;
        let (_event_loop, window) = test_window([320, 240]);
        let renderer = game.new_renderer(window).unwrap();
        game.set_renderer(renderer);
        game.show_title(title);

        let [r, g, b] = corner_of(game, frame);
        assert!(r > 0 && g == 0 && b == 0, "title corner was {r}, {g}, {b}");
    });
}
