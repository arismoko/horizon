//! Which structures contain a position, from the chunk's structure references and the generator's cached starts.

/// The structures reaching `pos` (id, inside a piece) over the world's chunks and, asked only when a chunk refers to a structure, the terrain that locates the starts.
pub fn structures_reaching_in(
    world: &sunrise_level::world::World,
    locator: &dyn Fn() -> Option<std::sync::Arc<sunrise_sim::vanillagen::VanillaTerrain>>,
    pos: [i32; 3],
    wanted: &dyn Fn(&str) -> bool,
) -> Vec<(String, bool)> {
    let Some(chunk) = world.chunks.get(&(pos[0] >> 4, pos[2] >> 4)) else {
        return Vec::new();
    };
    if chunk.structure_refs.is_empty() {
        return Vec::new();
    }
    let Some(locator) = locator() else {
        return Vec::new();
    };
    let mut out: Vec<(String, bool)> = Vec::new();
    for (id, (sx, sz)) in &chunk.structure_refs {
        if !wanted(id) {
            continue;
        }
        for start in locator.starts_at(*sx, *sz) {
            if *start.name != **id || !start.bounding_box.is_inside(pos) {
                continue;
            }
            let piece = start.pieces.iter().any(|p| p.bounding_box().is_inside(pos));
            match out.iter_mut().find(|(n, _)| *n == start.name) {
                Some(e) => e.1 |= piece,
                None => out.push((start.name.clone(), piece)),
            }
        }
    }
    out
}
