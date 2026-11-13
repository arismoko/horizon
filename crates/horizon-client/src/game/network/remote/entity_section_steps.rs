use super::Game;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    pub(super) fn remote_entity_section_step(&mut self, id: u32, from: Vec3, to: Vec3) {
        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }

}

pub(super) fn player_before(game: &Game<'_>) -> Option<Vec3> {
    game.player.as_ref().map(|player| player.position)
}

pub(super) fn player_after(game: &mut Game<'_>, from: Option<Vec3>) {
    if let (Some(from), Some(to), Ok(id)) = (
        from,
        game.player.as_ref().map(|player| player.position),
        u32::try_from(game.own_entity_id()),
    ) && from != to
    {
        game.remote_entity_section_step(id, from, to);
    }
}

pub(super) fn tick_items(game: &mut Game<'_>, entered: &mut Vec<horizon_client::entities::ItemSplash>) {
    let mut items = game.dropped.lift();
    let mut i = 0;
    while i < items.len() {
        let (_, discarded) = game.tick_item_with_block_contacts(
            &mut items[i],
            None,
            &[],
            true,
            &mut |splash| entered.push(splash),
        );
        let from = items[i].previous;
        let to = items[i].position;
        if !discarded {
            game.record_dropped_item_step(items[i].id, from, to);
        }
        if discarded {
            items.remove(i);
        } else {
            i += 1;
        }
    }
    game.put_back_dropped(items);
}
