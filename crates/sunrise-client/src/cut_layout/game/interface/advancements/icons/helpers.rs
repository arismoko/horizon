use std::path::PathBuf;
use sunrise_client_render::hud::{GuiScale, HudBatch};

pub fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sunrise-client/tests/fixtures/advancement-icons")
}

pub fn batch() -> HudBatch {
    HudBatch::new(
        [640, 480],
        GuiScale {
            requested: 1,
            unicode: false,
        },
    )
}

pub fn glint_vertices(batch: &HudBatch, x: f32, y: f32) -> usize {
    batch
        .vertices
        .iter()
        .filter(|v| {
            (2.0..4.0).contains(&v.uv[0])
                && (x..=x + 16.0).contains(&v.pos[0])
                && (y..=y + 16.0).contains(&v.pos[1])
        })
        .count()
}

pub fn assert_icon(batch: &HudBatch, x: f32, y: f32, foil: bool, damaged: bool) {
    assert_eq!(glint_vertices(batch, x, y) > 0, foil, "glint at ({x}, {y})");
    // Count 3 is drawn in the icon's lower-right corner, not a default count 1.
    assert!(
        batch.vertices.iter().any(|v| {
            v.text == 1
                && (x + 9.0..=x + 17.0).contains(&v.pos[0])
                && (y + 9.0..=y + 18.0).contains(&v.pos[1])
        }),
        "count decoration at ({x}, {y})"
    );
    // A wooden pickaxe has 59 durability: damage 7 leaves an 11-pixel bar.
    let bar = batch.vertices.windows(6).any(|vertices| {
        let min_x = vertices
            .iter()
            .map(|v| v.pos[0])
            .fold(f32::INFINITY, f32::min);
        let max_x = vertices
            .iter()
            .map(|v| v.pos[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = vertices
            .iter()
            .map(|v| v.pos[1])
            .fold(f32::INFINITY, f32::min);
        let max_y = vertices
            .iter()
            .map(|v| v.pos[1])
            .fold(f32::NEG_INFINITY, f32::max);
        [min_x, min_y, max_x, max_y] == [x + 2.0, y + 13.0, x + 13.0, y + 14.0]
    });
    assert_eq!(bar, damaged, "damage decoration at ({x}, {y})");
}
