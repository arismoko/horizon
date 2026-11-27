use crate::game::ClientGame;
use mod_api::Pose;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::entity_sizes_common::synced_pose;
use sunrise_sim::pose::SizeAnswer;

impl ClientGame<'_> {
    /// What the client copy `id`'s size refresh computes now: its pose and vanilla size, with
    /// the answer it holds.
    pub(in crate::game::client_side) fn client_copy_size(
        &mut self,
        id: u32,
        pose: Option<Pose>,
    ) -> Option<(Pose, [f32; 3], Option<SizeAnswer>)> {
        if let Some(i) = self.common.mob_index.find(&self.common.mobs, id) {
            let m = &self.common.mobs[i];
            let pose = synced_pose(m);
            return Some((pose, m.vanilla_dims_in(pose), m.size_answer));
        }
        let remote = self.client.remote.as_ref()?;
        let previous = remote.entities.get(&(id as i32))?.size_answer;
        let (kind, _) = remote.tracked_entity(id as i32)?;
        let pose = pose
            .or(previous.map(|answer| answer.pose))
            .unwrap_or(Pose::Standing);
        Some((pose, self.client_object_dimensions(id, kind)?, previous))
    }

    pub(in crate::game::client_side) fn set_client_copy_size(
        &mut self,
        id: u32,
        answer: SizeAnswer,
    ) {
        if let Some(m) = self.common.mob_index.get_mut(&mut self.common.mobs, id) {
            m.size_answer = Some(answer);
        } else if let Some(row) = self
            .client
            .remote
            .as_mut()
            .and_then(|remote| remote.entities.get_mut(&(id as i32)))
        {
            row.size_answer = Some(answer);
        }
    }

    /// `refreshDimensions` on a client's copy `id` of a server entity.
    pub(in crate::game) fn native_client_copy_size(&mut self, id: u32, pose: Option<Pose>) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        if let Some((pose, vanilla, previous)) = self.client_copy_size(id, pose)
            && let Some(answer) = self.ask_entity_size(id, pose, vanilla, previous)
        {
            self.set_client_copy_size(id, answer);
        }
    }

    /// `EntityEvent.Size` from the constructor of a client's copy `id` of type `kind`.
    pub(in crate::game) fn native_constructing_size(
        &self,
        id: u32,
        kind: &str,
    ) -> Option<SizeAnswer> {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return None;
        }
        let vanilla = super::entity_type_rows::entity_type_rows(&self.common.options.resources)
            .dimensions(kind)?;
        self.ask_entity_size(id, Pose::Standing, vanilla, None)
    }

    /// Keep the constructor's answer on the copy it was asked for, now that it exists.
    pub(in crate::game) fn native_constructed_size(&mut self, id: u32, answer: Option<SizeAnswer>) {
        if let Some(answer) = answer {
            self.set_client_copy_size(id, answer);
        }
    }

    /// The pose and vanilla size of the client copy `id` before a synched value lands.
    pub(in crate::game) fn synched_size_key(&mut self, id: u32) -> Option<(Pose, [f32; 3])> {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return None;
        }
        self.client_copy_size(id, None)
            .map(|(pose, vanilla, _)| (pose, vanilla))
    }

    /// After a synched value `(index, value)` landed on the client copy `id`.
    pub(in crate::game) fn native_synched_size(
        &mut self,
        id: u32,
        before: Option<(Pose, [f32; 3])>,
        index: u8,
        value: &sunrise_protocol::entity_data::Value,
    ) {
        let Some(before) = before else {
            return;
        };
        let pose = match (index, value) {
            // `Pose.STREAM_CODEC`: an unknown id is standing, as `from_id` reads it.
            (6, sunrise_protocol::entity_data::Value::Id(pose)) => Some(Pose::from_id(*pose)),
            _ => None,
        };
        if self
            .client_copy_size(id, pose)
            .is_some_and(|(pose, vanilla, _)| (pose, vanilla) != before)
        {
            self.native_client_copy_size(id, pose);
        }
    }

    /// Resolve candidate sizes with the read Host before borrowing the body for motion.
    pub(in crate::game) fn native_local_player_fit_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        let Some(id) = self
            .client
            .remote
            .as_ref()
            .map(|remote| remote.entity_id as u32)
        else {
            return;
        };
        let Some(answers) = self
            .common
            .player
            .as_ref()
            .map(|body| self.body_fit_sizes(id, body, body.size_answer))
        else {
            return;
        };
        if let Some(body) = self.common.player.as_mut() {
            body.pose_fit_sizes = answers;
        }
    }

    /// The client's own body, after its tick (`LocalPlayer.tick` set its pose).
    pub(in crate::game) fn native_local_player_size(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        let Some(id) = self
            .client
            .remote
            .as_ref()
            .map(|remote| remote.entity_id as u32)
        else {
            return;
        };
        let Some(answer) = self
            .common
            .player
            .as_ref()
            .and_then(|body| self.ask_body_size(id, body))
        else {
            return;
        };
        if let Some(body) = self.common.player.as_mut() {
            sunrise_gameplay_common::game::CommonGame::apply_body_size(body, answer);
        }
    }

    /// The vanilla size of a client's copy `id` of a non-mob entity of type `kind`
    /// (`Entity.getDimensions(pose)`): its type's, or what its synched values make it.
    pub(in crate::game) fn client_object_dimensions(
        &self,
        id: u32,
        kind: &str,
    ) -> Option<[f32; 3]> {
        if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id) {
            return Some(a.default_dimensions());
        }
        if let Some(i) = self.client.interactions.get(&(id as i32)) {
            // Interaction.getDimensions: EntityDimensions.scalable(width, height).
            return Some([i.width, i.height, i.height * 0.85]);
        }
        if let Some(c) = self.client.remote_clouds.iter().find(|c| c.id == id) {
            // AreaEffectCloud.getDimensions: scalable(radius * 2, 0.5).
            return Some([c.radius * 2.0, 0.5, 0.5 * 0.85]);
        }
        super::entity_type_rows::entity_type_rows(&self.common.options.resources).dimensions(kind)
    }

    /// Size a client's copy `id` takes from its `entity_size` answer while its vanilla size holds.
    pub(in crate::game::client_side) fn client_copy_answer(&self, id: u32) -> Option<[f32; 3]> {
        let remote = self.client.remote.as_ref()?;
        let answer = remote.entities.get(&(id as i32))?.size_answer?;
        let size = answer.size?;
        let (kind, _) = remote.tracked_entity(id as i32)?;
        (self.client_object_dimensions(id, kind)? == answer.vanilla).then_some(size)
    }

    /// Bounding box of a client's copy `id` at `position`, preserving `vanilla`'s bottom.
    pub(in crate::game::client_side) fn client_copy_box_inner(
        &self,
        id: u32,
        position: Vec3,
        vanilla: Aabb,
    ) -> Aabb {
        let Some([width, height, _]) = self.client_copy_answer(id) else {
            return vanilla;
        };
        let half = f64::from(width / 2.0);
        let bottom = vanilla.min_position().y;
        Aabb::new(
            Vec3::new(position.x - half, bottom, position.z - half),
            Vec3::new(
                position.x + half,
                bottom + f64::from(height),
                position.z + half,
            ),
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    /// The F3+B outline's white edges: their lowest and highest corner.
    pub fn white_outline(game: &mut crate::game::ClientGame<'_>) -> ([f64; 3], [f64; 3]) {
        // F3+B, on entries with no file behind them: nothing is saved.
        game.client.debug_entries.0 = sunrise_client::debug_entries::DebugEntries::default();
        game.client
            .debug_entries
            .0
            .toggle_status("entity_hitboxes", false);
        let gizmos = game.debug_gizmos(1.0, [0.0; 3]);
        let ends: Vec<[f64; 3]> = gizmos
            .lines
            .iter()
            .filter(|line| line.color == sunrise_client::gizmos::WHITE)
            .flat_map(|line| [line.start, line.end])
            .collect();
        assert_eq!(ends.len(), 24, "one white cuboid");
        let extent = |pick: fn(f64, f64) -> f64, from: f64| {
            std::array::from_fn(|axis| ends.iter().map(|end| end[axis]).fold(from, pick))
        };
        (
            extent(f64::min, f64::INFINITY),
            extent(f64::max, f64::NEG_INFINITY),
        )
    }
}

use sunrise_gameplay_common::game::entity_sizes_common::EntitySizesCommonGameExt1 as _;
