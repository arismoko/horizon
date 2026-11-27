use crate::game::ClientGame;
use sunrise_core::geometry::Vec3;

#[cfg(feature = "client")]
pub(in crate::game) fn player_teleport(
    game: &mut crate::game::ClientGame<'_>,
    had_player: bool,
    from: Vec3,
    to: Vec3,
) {
    if had_player && let Ok(id) = u32::try_from(game.own_entity_id()) {
        game.remote_entity_section_step(id, from, to);
    }
}

#[cfg(feature = "client")]
pub(in crate::game) fn tick_items(
    game: &mut crate::game::ClientGame<'_>,
    entered: &mut Vec<sunrise_sim::entities::ItemSplash>,
) {
    let mut items = game.dropped.lift();
    let mut i = 0;
    while i < items.len() {
        let (_, discarded) = game.tick_item_with_block_contacts(
            &mut items[i],
            None,
            &[],
            true,
            None,
            &mut |splash| entered.push(splash),
        );
        let from = items[i].previous;
        let to = items[i].position;
        if !discarded {
            let live = game.entity_section_step_visible(
                items[i].id,
                [from.x, from.y, from.z],
                [to.x, to.y, to.z],
            );
            if !live {
                items.remove(i);
                continue;
            }
        }
        if discarded {
            items.remove(i);
        } else {
            i += 1;
        }
    }
    game.put_back_dropped(items);
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `ClientLevel.isTickingEntity`: players always tick; other entities tick while their chunk is loaded.
    pub(in crate::game::client_side) fn remote_entity_is_ticking(&self, id: i32) -> bool {
        let Some(r) = self.client.remote.as_ref() else {
            return true;
        };
        if r.entity_id == id || r.level_players.contains_key(&id) {
            return true;
        }
        let pos = if let Some(m) = self.common.mob_index.get_ref(&self.common.mobs, id as u32) {
            [m.position.x, m.position.z]
        } else if let Some(i) = self.client.interactions.get(&id) {
            [i.position.x, i.position.z]
        } else if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id as u32) {
            [a.position.x, a.position.z]
        } else if let Some(b) = self.common.boats.iter().find(|b| b.id == id as u32) {
            [b.position.x, b.position.z]
        } else if let Some(c) = self.common.carts.iter().find(|c| c.id == id as u32) {
            [c.position.x, c.position.z]
        } else if let Some(t) = r.entities.get(&id) {
            [t.position[0], t.position[2]]
        } else {
            return false;
        };
        self.common
            .world
            .chunks
            .contains_key(&((pos[0].floor() as i32) >> 4, (pos[1].floor() as i32) >> 4))
    }
    pub(in crate::game) fn remote_entity_section_step(&mut self, id: u32, from: Vec3, to: Vec3) {
        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }
    pub(in crate::game) fn remote_move_entity_and_file(
        &mut self,
        id: i32,
        path: Option<super::Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        self.move_entity(id, path, rotation, on_ground, super::MovePacket::Move);
        self.entity_section_moved_id(id as u32);
    }
    /// The driven vehicle put at `position` with `rotation`, its section crossing filed.
    pub(in crate::game::client_side) fn remote_move_vehicle_and_file(
        &mut self,
        position: [f64; 3],
        rotation: (f32, f32),
    ) {
        self.remote_move_vehicle(position, rotation);
        let boat = self.common.riding.filter(|&id| self.controls_boat(id));
        if let Some(id) = self.driven_mount().map(|m| m.id).or(boat) {
            self.entity_section_moved_id(id);
        }
    }
    pub(in crate::game::client_side) fn remote_position_sync_and_file(
        &mut self,
        id: i32,
        path: super::Path,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) {
        let packet = if self.remote_entity_is_ticking(id) {
            super::MovePacket::PositionSync
        } else {
            super::MovePacket::PositionSyncSnap
        };
        self.move_entity(id, Some(path), Some((yaw, pitch)), on_ground, packet);
        self.entity_section_moved_id(id as u32);
    }
}

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
