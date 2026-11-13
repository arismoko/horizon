use super::*;

impl Game<'_> {
    /// `EntityEvent.EnteringSection`: `entity` is in the section `new_section`, no longer in
    /// `old_section`; `chunk_changed` when their x or z differ (`modding/entity_sections.rs`).
    pub(in crate::game) fn native_entity_entering_section(
        &mut self,
        entity: EntityRef,
        old_section: [i32; 3],
        new_section: [i32; 3],
        chunk_changed: bool,
    ) {
        self.fire_entity_entering_section(entity, old_section, new_section, chunk_changed);
    }
}
