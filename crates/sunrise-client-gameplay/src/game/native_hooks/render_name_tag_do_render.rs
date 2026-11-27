use crate::game::ClientGame;
use crate::game::native_block_entity_draw::BlockEntityDraw;
use mod_api::NameTagLine;

impl ClientGame<'_> {
    /// Before one line of a name tag is drawn (NeoForge's `RenderNameTagEvent.DoRender`), with a.
    pub(in crate::game) fn native_render_name_tag_do_render(
        &self,
        entity_id: u32,
        kind: &str,
        line: NameTagLine,
        content: &str,
        attachment: [f64; 3],
        partial_tick: f32,
        draw: &mut BlockEntityDraw<'_>,
        canceled: &mut bool,
    ) {
        self.fire_render_name_tag_do_render(
            entity_id,
            kind,
            line,
            content,
            attachment,
            partial_tick,
            draw,
            canceled,
        );
    }
}
