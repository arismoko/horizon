use super::Game;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    pub(super) fn remote_entity_section_step(&mut self, id: u32, from: Vec3, to: Vec3) {
        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }

    pub(super) fn remote_entity_section_moved(&mut self, id: i32) {
        if let Ok(id) = u32::try_from(id) {
            self.entity_section_moved_id(id);
        }
    }

    pub(super) fn remote_entity_section_joined(&mut self, id: i32, at: [f64; 3]) {
        if let Ok(id) = u32::try_from(id) {
            self.entity_section_joined(id, at);
        }
    }
}
