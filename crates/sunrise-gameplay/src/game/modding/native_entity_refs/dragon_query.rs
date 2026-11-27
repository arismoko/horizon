//! The separately stored dragon participates in Level.getEntities; parts follow the walk.
use super::*;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn visit_dragon_query_parts(
        &self,
        bounds: Aabb,
        visit_id: &mut impl FnMut(u32) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let Some(dragon) = self.common.dragon.as_ref().filter(|d| d.id != 0) else {
            return ControlFlow::Continue(());
        };
        let mut cursor = None;
        while let Some(id) = self.common.dragon.next_part(&mut cursor) {
            if let Some(part) =
                sunrise_gameplay_common::game::dragon_common::dragon_part_index(dragon.id, id)
                && bounds.intersects(dragon.part_geometry(part).1)
            {
                visit_id(id)?;
            }
        }
        ControlFlow::Continue(())
    }
}
