//! `WorldDrawContext::name_tag` (#4171): a mod's label hides behind a wall unless it is
//! see-through, shrinks with distance as a name tag does, supports deep in-memory text
//! or a position that is not finite, draws as vanilla's name tags in its light and the
//! player's text background opacity.
use super::native_entity_refs::tests::with_client_game;
use crate::game::host::draw::GameWorldDrawContext;
use glam::{DVec3, Mat4, Vec3};
use mod_api::{DrawError, Hooks, NameTagOptions, Text, WorldDrawContext};
use sunrise_client::block_entity_render::vertex_light;
use sunrise_client::{mesher::Vertex, world_geometry::WorldGeometry};

#[path = "world_name_tag_tests/scene.rs"]
mod scene;

/// Four blocks in front of the label's anchor (0.5 above (0.5, 70, 0.5)), facing south (+Z).
const CAMERA: [f64; 3] = [0.5, 70.5, -3.5];

/// The view the tests draw with: the camera looking south, reversed depth as the game's.
fn view_proj() -> Mat4 {
    let projection = Mat4::perspective_rh(70f32.to_radians(), 1.0, 512.0, 0.05);
    projection * Mat4::look_to_rh(Vec3::ZERO, Vec3::Z, Vec3::Y)
}

/// The game's camera at [`CAMERA`]'s facing: south, first person.
fn face_south(game: &mut crate::game::ClientGame<'_>) {
    game.common.yaw = 0.0;
    game.common.pitch = 0.0;
    game.client.posed_rotation = None;
    game.client.camera_type = sunrise_level::camera_math::CameraType::FirstPerson;
}

/// One view's labels at `at`, the game's camera facing south: what it draws with the name
/// tags, then the see-through copies.
fn labels(
    game: &mut crate::game::ClientGame<'_>,
    at: [f64; 3],
    text: &Text,
    options: NameTagOptions,
) -> (Result<(), DrawError>, [Vec<Vertex>; 2]) {
    face_south(game);
    let mut draw = WorldGeometry::default();
    draw.reset(CAMERA, Mat4::IDENTITY, [0.5, 0.5]);
    let result = GameWorldDrawContext::new(game, &mut draw).name_tag(at, text, options);
    (result, draw.labels().map(<[Vertex]>::to_vec))
}

/// The alphas of a label's text quads and of its backdrop quads (black), each sorted and
/// without repeats.
fn alphas(vertices: &[Vertex]) -> (Vec<u32>, Vec<u32>) {
    let (mut text, mut backdrop) = (Vec::new(), Vec::new());
    for v in vertices {
        let out = if v.color & 0xFF_FFFF == 0 {
            &mut backdrop
        } else {
            &mut text
        };
        out.push(v.color >> 24);
    }
    for list in [&mut text, &mut backdrop] {
        list.sort_unstable();
        list.dedup();
    }
    (text, backdrop)
}

/// `vertices` with every light cleared, as bytes: two labels that differ only in light match.
fn without_light(vertices: &[Vertex]) -> Vec<u8> {
    let cleared: Vec<Vertex> = vertices.iter().map(|v| Vertex { light: 0, ..*v }).collect();
    bytemuck::cast_slice(&cleared).to_vec()
}

fn bytes(vertices: &[Vertex]) -> &[u8] {
    bytemuck::cast_slice(vertices)
}

/// An opaque grey wall two blocks in front of the camera, in the white texel.
fn wall(game: &crate::game::ClientGame<'_>) -> Vec<Vertex> {
    let white = game.client.gui.atlas.sprites["sunrise/white"];
    let uv = [(white.u0 + white.u1) * 0.5, (white.v0 + white.v1) * 0.5];
    // Counterclockwise as seen from the camera.
    [[4.0, -4.0], [-4.0, -4.0], [-4.0, 4.0], [4.0, 4.0]]
        .map(|[x, y]| Vertex {
            pos: [x, y, 2.0],
            uv,
            layer: 0,
            color: 0xFF80_8080,
            light: vertex_light(0xF0),
            normal: [0; 4],
            glint_uv: [0.0; 2],
        })
        .to_vec()
}

/// The offscreen pixels of `world` (the entity pass) then `see_through` (no depth test), under
/// a lightmap whose brightness follows the light level, so light shows in the pixels.
fn pixels(game: &crate::game::ClientGame<'_>, world: &[Vertex], see_through: &[Vertex]) -> Vec<u8> {
    use sunrise_client::gfx::entity_pixel_test::{graded_lightmap, render_see_through_lit};
    render_see_through_lit(
        world,
        see_through,
        [game.client.gui.atlas.width, game.client.gui.atlas.height],
        &game.client.gui.atlas.pixels(),
        view_proj(),
        &graded_lightmap(),
    )
    .expect("software Vulkan label pixels")
}

fn brightness(pixels: &[u8]) -> u64 {
    pixels.iter().map(|&c| u64::from(c)).sum()
}

#[test]
#[ignore = "daily"]
fn a_label_hides_behind_a_wall_unless_it_is_see_through() {
    with_client_game(|game| {
        let text = Text::literal("Label");
        let at = [0.5, 70.0, 0.5];
        let (ok, [world, through]) =
            labels(game, at, &text, NameTagOptions::new(false, true, 0xF0));
        assert_eq!(ok, Ok(()));
        assert!(!world.is_empty() && through.is_empty());
        let empty = pixels(game, &[], &[]);
        assert_ne!(pixels(game, &world, &[]), empty, "visible without a wall");
        // An alpha under 26 draws no text (its fragments are discarded); 26 does. `exact_light`
        // has no effect without `see_through`.
        let faded = |game: &mut crate::game::ClientGame<'_>, alpha| {
            let options = NameTagOptions::new(false, false, 0xF0).with_faded_alpha(alpha);
            let copy = labels(game, [0.5, 70.0, 0.5], &text, options).1[0].clone();
            pixels(game, &copy, &[])
        };
        assert_eq!(faded(game, 25), empty);
        assert_ne!(faded(game, 26), empty);
        let plain = NameTagOptions::new(false, true, 0x00);
        assert_eq!(
            bytes(&labels(game, [0.5, 70.0, 0.5], &text, plain.with_exact_light()).1[0]),
            bytes(&labels(game, [0.5, 70.0, 0.5], &text, plain).1[0]),
        );
        let wall = wall(game);
        let only_wall = pixels(game, &wall, &[]);
        let behind = [wall.clone(), world].concat();
        assert_eq!(
            pixels(game, &behind, &[]),
            only_wall,
            "hidden behind the wall"
        );

        let (ok, [world, through]) = labels(game, at, &text, NameTagOptions::new(true, true, 0xF0));
        assert_eq!(ok, Ok(()));
        assert!(!world.is_empty() && !through.is_empty());
        let behind = [wall, world].concat();
        let shown = pixels(game, &behind, &through);
        assert_ne!(shown, only_wall, "the dim copy over the wall");
    });
}

/// Default options without `see_through`: one depth-tested translucent copy, the faded text
/// (0x7F) on the backdrop (0x3F), in the caller's light exactly: light 0 is not raised to 2.
#[test]
#[ignore = "daily"]
fn without_see_through_a_label_is_one_translucent_copy_in_the_callers_light() {
    with_client_game(|game| {
        let text = Text::literal("Label");
        let drawn = |game: &mut crate::game::ClientGame<'_>, light| {
            let options = NameTagOptions::new(false, true, light);
            let (ok, [world, through]) = labels(game, [0.5, 70.0, 0.5], &text, options);
            assert_eq!(ok, Ok(()));
            assert!(through.is_empty(), "one copy");
            world
        };
        let [dark, two, full] = [0x00, 0x22, 0xFF].map(|light| drawn(game, light));
        assert_eq!(alphas(&dark), (vec![0x7F], vec![0x3F]));
        for (label, light) in [(&dark, 0x00), (&two, 0x22), (&full, 0xFF)] {
            assert!(
                label.iter().all(|v| v.light == vertex_light(light)),
                "light {light:#04x} as given"
            );
            assert!(label.iter().all(|v| v.layer == 0));
        }
        // The same label but for its light, which alone sets its brightness.
        assert_eq!(without_light(&dark), without_light(&full));
        let [dark, two, full] =
            [&dark, &two, &full].map(|label| brightness(&pixels(game, label, &[])));
        assert!(dark < two && two < full, "{dark} < {two} < {full}");
        let wall = wall(game);
        let label = drawn(game, 0x00);
        assert_eq!(
            pixels(game, &[wall.clone(), label].concat(), &[]),
            pixels(game, &wall, &[]),
            "depth tested: hidden behind the wall"
        );
    });
}

/// `see_through` with light 0: the copy in the world is opaque white with no backdrop at the
/// brightness of light 2; the faded copy (0x7F on 0x3F) is unlit and does not change with
/// light; behind a wall only the faded copy and its backdrop show.
#[test]
#[ignore = "daily"]
fn a_see_through_label_is_lit_at_least_2_in_the_world_and_unlit_over_it() {
    with_client_game(|game| {
        let text = Text::literal("Label");
        let drawn = |game: &mut crate::game::ClientGame<'_>, light| {
            let options = NameTagOptions::new(true, true, light);
            let (ok, copies) = labels(game, [0.5, 70.0, 0.5], &text, options);
            assert_eq!(ok, Ok(()));
            copies
        };
        let [world, through] = drawn(game, 0x00);
        assert_eq!(alphas(&world), (vec![0xFF], vec![]), "opaque, no backdrop");
        assert!(world.iter().all(|v| v.light == vertex_light(0x22)));
        assert_eq!(alphas(&through), (vec![0x7F], vec![0x3F]));
        assert!(through.iter().all(|v| v.light == 0 && v.layer == 2));
        let [world_two, _] = drawn(game, 0x22);
        assert_eq!(bytes(&world), bytes(&world_two), "light 0 drawn as light 2");
        assert_eq!(pixels(game, &world, &[]), pixels(game, &world_two, &[]));
        let [_, through_full] = drawn(game, 0xFF);
        assert_eq!(
            bytes(&through),
            bytes(&through_full),
            "the faded copy ignores light"
        );
        assert_eq!(
            pixels(game, &[], &through),
            pixels(game, &[], &through_full)
        );
        // `with_exact_light`: the copy in the world is lit by light 0 itself, darker than
        // light 2's, and nothing else changes.
        let exact = {
            let options = NameTagOptions::new(true, true, 0x00).with_exact_light();
            let (ok, copies) = labels(game, [0.5, 70.0, 0.5], &text, options);
            assert_eq!(ok, Ok(()));
            copies
        };
        assert!(exact[0].iter().all(|v| v.light == vertex_light(0x00)));
        assert_eq!(
            bytes(&exact[1]),
            bytes(&through),
            "the faded copy is unchanged"
        );
        assert_eq!(without_light(&exact[0]), without_light(&world));
        let (dim, lit) = (
            brightness(&pixels(game, &exact[0], &[])),
            brightness(&pixels(game, &world, &[])),
        );
        // A smoke check of the rendering; the vertex light assertions above pin the contract.
        assert!(dim < lit, "{dim} < {lit}");
        // `with_faded_alpha`: the faded text at alpha 0x20 whichever copy it is, on the same
        // backdrop, and fainter on screen than at the default 0x7F.
        let faint = {
            let options = NameTagOptions::new(true, true, 0x00).with_faded_alpha(0x20);
            let (ok, copies) = labels(game, [0.5, 70.0, 0.5], &text, options);
            assert_eq!(ok, Ok(()));
            copies
        };
        assert_eq!(alphas(&faint[1]), (vec![0x20], vec![0x3F]));
        assert_eq!(
            bytes(&faint[0]),
            bytes(&world),
            "the copy in the world is unchanged"
        );
        let single = {
            let options = NameTagOptions::new(false, true, 0x00).with_faded_alpha(0x20);
            labels(game, [0.5, 70.0, 0.5], &text, options).1[0].clone()
        };
        assert_eq!(alphas(&single), (vec![0x20], vec![0x3F]));
        let (faint_light, normal) = (
            brightness(&pixels(game, &[], &faint[1])),
            brightness(&pixels(game, &[], &through)),
        );
        assert!(faint_light < normal, "{faint_light} < {normal}");
        let wall = wall(game);
        let only_wall = pixels(game, &wall, &[]);
        let behind = pixels(game, &[wall.clone(), world].concat(), &through);
        assert_eq!(
            behind,
            pixels(game, &wall, &through),
            "behind the wall only the faded copy and its backdrop"
        );
        assert_ne!(behind, only_wall);
    });
}

/// The text background option moved off chat only, at 0.5: backdrop 127, faded text 159, for
/// the one translucent copy and for the see-through copy alike.
#[test]
#[ignore = "daily"]
fn the_text_background_opacity_sets_the_backdrop_and_the_faded_text() {
    with_client_game(|game| {
        use sunrise_client::options_file::V;
        game.client
            .settings
            .file
            .set("backgroundForChatOnly", V::Bool(false));
        game.client.settings.text_background_opacity = 0.5;
        let text = Text::literal("Label");
        let at = [0.5, 70.0, 0.5];
        let (ok, [world, _]) = labels(game, at, &text, NameTagOptions::new(false, true, 0xF0));
        assert_eq!(ok, Ok(()));
        assert_eq!(alphas(&world), (vec![159], vec![127]));
        let (ok, [world, through]) = labels(game, at, &text, NameTagOptions::new(true, true, 0xF0));
        assert_eq!(ok, Ok(()));
        assert_eq!(alphas(&world), (vec![0xFF], vec![]));
        assert_eq!(alphas(&through), (vec![159], vec![127]));
        // Chat only again: the default 0.25 whatever the slider says.
        game.client
            .settings
            .file
            .set("backgroundForChatOnly", V::Bool(true));
        let (_, [world, _]) = labels(game, at, &text, NameTagOptions::new(false, true, 0xF0));
        assert_eq!(alphas(&world), (vec![0x7F], vec![0x3F]));
    });
}

/// Labels grow past the initial 4,096 quad pre-reservation hint alongside plain quads,
/// and newly rasterized glyphs upload their atlas patches.
#[test]
#[ignore = "daily"]
fn labels_grow_past_initial_quad_hint_and_upload_glyphs() {
    with_client_game(|game| {
        face_south(game);
        // The default font's runes are already packed bitmap glyphs, not runtime uploads, and
        // the prepared resources carry no Unifont. Give the game a font whose runes are hex
        // glyphs and an empty cache, so this layout must rasterize them.
        let mut hex = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        hex.start_file("runes.hex", zip::write::SimpleFileOptions::default())
            .unwrap();
        let rows = "3C".repeat(16);
        let glyphs: String = ["16A0", "16A2", "16A6"]
            .iter()
            .map(|rune| format!("{rune}:{rows}\n"))
            .collect();
        std::io::Write::write_all(&mut hex, glyphs.as_bytes()).unwrap();
        let hex = hex.finish().unwrap().into_inner();
        let source = sunrise_assets::resource_source::ResourceSource::indexed(
            [
                (
                    "assets/minecraft/font/default.json",
                    br#"{"providers":[{"type":"unihex","hex_file":"minecraft:font/runes.zip"}]}"#
                        .to_vec(),
                ),
                ("assets/minecraft/font/runes.zip", hex),
            ]
            .map(|(path, bytes)| {
                (
                    path.to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(bytes),
                )
            })
            .into(),
        );
        let atlas = &mut game.client.gui.atlas;
        atlas.font = sunrise_client::font::Font::load(
            &source,
            &atlas.sprites,
            &[],
            (atlas.width, atlas.height),
            (atlas.height / 2, atlas.height / 2),
            atlas.sprites["sunrise/white"],
        );
        let text = Text::literal("\u{16A0}\u{16A2}\u{16A6}");
        let at = [0.5, 70.0, 0.5];
        let options = NameTagOptions::new(false, true, 0xF0);
        let mut draw = WorldGeometry::default();
        draw.reset(CAMERA, Mat4::IDENTITY, [0.5, 0.5]);
        let quad = mod_api::WorldQuad::new([[0.5, 70.5, 0.0]; 4], !0, mod_api::WorldDepth::Tested);
        draw.quads(&vec![
            quad;
            sunrise_client::gfx::world_primitives::DEFAULT_CAPACITY
                as usize
        ])
        .unwrap();
        game.client.text_patches.borrow_mut().clear();
        let accepted = GameWorldDrawContext::new(game, &mut draw).name_tag(at, &text, options);
        assert_eq!(accepted, Ok(()));
        let queued: Vec<[u32; 4]> = game
            .client
            .text_patches
            .borrow()
            .iter()
            .map(|p| [p.x, p.y, p.width, p.height])
            .collect();
        assert!(!queued.is_empty(), "new glyphs upload their atlas patches");
        let size = [
            game.client.gui.atlas.width as f32,
            game.client.gui.atlas.height as f32,
        ];
        let glyphs: Vec<&[Vertex; 4]> = draw.labels()[0]
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|quad| quad[0].color & 0xFF_FFFF != 0)
            .collect();
        assert!(!glyphs.is_empty());
        for quad in glyphs {
            // The centre of the glyph's texels, in atlas pixels.
            let [u, v] = quad
                .iter()
                .fold([0.0f32; 2], |[u, v], c| [u + c.uv[0], v + c.uv[1]]);
            let (x, y) = (u / 4.0 * size[0], v / 4.0 * size[1]);
            assert!(
                queued.iter().any(|&[px, py, w, h]| {
                    (px as f32..(px + w) as f32).contains(&x)
                        && (py as f32..(py + h) as f32).contains(&y)
                }),
                "glyph texel ({x}, {y}) is in a queued patch: {queued:?}"
            );
        }
    });
}

/// The shared name-tag tessellation's styles: bold counts in the centring width, plain text has
/// no drop shadow or outline (a quad a glyph), and a shadow is drawn only for a style that sets
/// a shadow colour.
#[test]
#[ignore = "daily"]
fn a_label_keeps_the_name_tag_styles() {
    with_client_game(|game| {
        let drawn = |game: &mut crate::game::ClientGame<'_>, style: mod_api::text::TextStyle| {
            let text = Text::literal("Bold").with_style(style);
            let options = NameTagOptions::new(false, false, 0xF0);
            let (ok, [world, _]) = labels(game, [0.5, 70.0, 0.5], &text, options);
            assert_eq!(ok, Ok(()));
            world
        };
        let width = |label: &[Vertex]| {
            let (lo, hi) = label
                .iter()
                .map(|v| v.pos[0])
                .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            hi - lo
        };
        let plain = drawn(game, Default::default());
        assert_eq!(plain.len() / 4, 4, "a quad a glyph: no shadow, no outline");
        let mut bold = mod_api::text::TextStyle::default();
        bold.bold = Some(true);
        let bold = drawn(game, bold);
        assert!(width(&bold) > width(&plain), "bold is wider");
        // Centred on the anchor whatever its width.
        for label in [&plain, &bold] {
            let (lo, hi) = label
                .iter()
                .map(|v| v.pos[0])
                .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            assert!(((lo + hi) / 2.0).abs() < 0.03, "centred: {lo}..{hi}");
        }
        let mut shadowed = mod_api::text::TextStyle::default();
        shadowed.shadow_color = Some(0xFF00_0000);
        assert!(
            drawn(game, shadowed).len() > plain.len(),
            "a shadow under the glyphs"
        );
    });
}

#[test]
#[ignore = "daily"]
fn labels_four_times_as_far_are_a_quarter_the_size_on_screen() {
    with_client_game(|game| {
        let text = Text::literal("Size");
        let width = |game: &mut crate::game::ClientGame<'_>, z: f64| {
            // The anchor is 0.5 below the label; the label `z` blocks ahead of the camera.
            let at = [CAMERA[0], CAMERA[1] - 0.5, CAMERA[2] + z];
            let (ok, [world, _]) = labels(game, at, &text, NameTagOptions::new(false, false, 0xF0));
            assert_eq!(ok, Ok(()));
            let xs = world
                .iter()
                .map(|v| view_proj().project_point3(Vec3::from(v.pos)).x);
            let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            hi - lo
        };
        let (near, far) = (width(game, 4.0), width(game, 16.0));
        assert!(near > 0.0);
        assert!((near / far - 4.0).abs() < 1e-3, "{near} / {far}");
    });
}

#[test]
#[ignore = "daily"]
fn a_deep_label_renders_and_non_finite_is_refused() {
    with_client_game(|game| {
        let mut deep = Text::literal("leaf");
        for _ in 1..520 {
            deep = Text::literal("").append(deep);
        }
        let options = NameTagOptions::new(true, true, 0xF0);
        let (ok, [world, through]) = labels(game, [0.5, 70.0, 0.5], &deep, options);
        assert_eq!(ok, Ok(()));
        assert!(!world.is_empty() && !through.is_empty());
        let mut bad_nbt = Text::literal("x");
        bad_nbt.metadata = sunrise_core::nbt::Tag::compound([(
            "deep",
            (0..512).fold(sunrise_core::nbt::Tag::Byte(1), |t, _| {
                sunrise_core::nbt::Tag::List(vec![t])
            }),
        )]);
        let (ok, drawn) = labels(game, [0.5, 70.0, 0.5], &bad_nbt, options);
        let (_, expected) = labels(game, [0.5, 70.0, 0.5], &Text::literal("x"), options);
        assert_eq!(ok, Ok(()));
        for (actual, expected) in drawn.iter().zip(&expected) {
            assert_eq!(
                bytemuck::cast_slice::<Vertex, u8>(actual),
                bytemuck::cast_slice::<Vertex, u8>(expected)
            );
        }
        for at in [[f64::NAN, 70.0, 0.5], [0.5, f64::INFINITY, 0.5]] {
            let (refused, drawn) = labels(game, at, &Text::literal("x"), options);
            assert_eq!(refused, Err(DrawError::NonFinite));
            assert!(drawn.iter().all(Vec::is_empty));
        }
        // Drawn half a block above its anchor: in the plane four blocks ahead, facing the camera.
        let (_, [world, _]) = labels(game, [0.5, 70.0, 0.5], &Text::literal("x"), options);
        let centre = world
            .iter()
            .fold(DVec3::ZERO, |sum, v| sum + Vec3::from(v.pos).as_dvec3())
            / world.len() as f64;
        assert!((centre.z - 4.0).abs() < 0.01, "{centre}");
    });
}

/// Adds labels to the actual frame via a native render hook.
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn submit_custom_geometry(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::SubmitCustomGeometryEvent<'_>,
    ) {
        scene::submit_custom_geometry(event.view, event.draw);
    }
}

/// The world pass and see-through pass receive the scene's labels.
fn scene_frame(game: &mut crate::game::ClientGame<'_>) -> ([Vec<Vertex>; 2], Vec<u32>) {
    face_south(game);
    game.common.world.game_time = scene::SCENE_TIME;
    game.client.see_through_vertices.clear();
    let mut draw = WorldGeometry::default();
    let mut entities = Vec::new();
    let frame_camera = mod_api::Camera {
        position: CAMERA,
        eye_height: 1.62,
        partial_tick: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_degrees: 70.0,
        near: 0.05,
        far: 128.0,
        view: Mat4::IDENTITY.to_cols_array(),
        projection: Mat4::IDENTITY.to_cols_array(),
        dimension: "minecraft:overworld".into(),
    };
    game.fill_world_geometry(
        false,
        Some(&frame_camera),
        CAMERA,
        Mat4::IDENTITY,
        0.0,
        false,
        &[],
        &mut draw,
        &mut entities,
    );
    let see_through = std::mem::take(&mut game.client.see_through_vertices);
    // Each call's result, from the scene's marker lines: `code` across, the call's index up.
    let mut results: Vec<(f32, u32)> = draw
        .batch()
        .lines
        .iter()
        .map(|line| (line[1], line[0].round() as u32))
        .collect();
    results.sort_by(|a, b| a.0.total_cmp(&b.0));
    let results = results.into_iter().map(|(_, code)| code).collect();
    ([entities, see_through], results)
}

#[test]
#[ignore = "daily"]
fn a_render_hook_can_add_labels_to_both_depth_passes() {
    let mods = vec![mod_api::ModEntry::new("native_labels", Box::new(Native))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let ([world, through], results) = scene_frame(game);
            assert_eq!(results, scene::RESULT_CODES);
            // Hidden (one copy), Through (two) and Bold (one) drew; NaN was refused.
            assert!(!world.is_empty() && !through.is_empty());
            assert!(world.len() > through.len());
        });
    });
}
