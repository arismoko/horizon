//! The lectern comparator signal.
use sunrise_core::position::BlockPos;
use sunrise_level::containers::BlockEntity;
use sunrise_world::registry::BlockStateId;

/// `LecternBlock.getAnalogOutputSignal` and
/// `LecternBlockEntity.getRedstoneSignal`: the state's `has_book` gates the
/// answer, and an entity with no book reads 14 (a page count of 0 is a page
/// progress of 1.0 with no book to add).
pub fn lectern_signal(
    world: &sunrise_level::world::World,
    registry: &sunrise_world::registry::BlockRegistry,
    state: BlockStateId,
    pos: BlockPos,
) -> Option<i32> {
    let s = registry.state(state)?;
    if s.name != "minecraft:lectern" {
        return None;
    }
    if s.properties.get("has_book").is_none_or(|v| v != "true") {
        return Some(0);
    }
    match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
        Some(BlockEntity::Lectern(l)) => Some(l.book.as_ref().map_or(14, |b| {
            let data = b.data();
            let count = data
                .and_then(|d| d.written.as_ref().map(|w| w.pages.len()))
                .or_else(|| data.map(|d| d.writable_pages.len()))
                .unwrap_or(0)
                .max(1);
            let f = if count > 1 {
                l.page as f32 / (count - 1) as f32
            } else {
                1.0
            };
            // `LecternBlockEntity.hasBook`: a `written_book_content` or
            // `writable_book_content` component, not just any stack. A book
            // item carries its content by default, so its id counts too.
            let has_book = matches!(
                &*b.item,
                "minecraft:writable_book" | "minecraft:written_book"
            ) || data
                .is_some_and(|d| d.written.is_some() || !d.writable_pages.is_empty());
            (f * 14.0).floor() as i32 + i32::from(has_book)
        })),
        _ => None,
    }
}
