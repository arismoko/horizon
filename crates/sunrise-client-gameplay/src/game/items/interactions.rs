//! Server `minecraft:interaction` entities (`Interaction`): invisible boxes a
//! server (Denizen, Nexo) uses for clickable areas. They are pickable, render
//! nothing, and an interact ends at them: `SUCCESS` when they respond (the arm
//! swings), `CONSUME` otherwise (the click still goes to the server, nothing
//! behind them is used).
use crate::game::ClientGame;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::interactions_common::RemoteInteraction;
use sunrise_protocol::entity_data::Value;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// An `Interaction` keeps `Entity`'s no-op interpolation handler: the
    /// server's position replaces its own at once.
    pub(in crate::game::client_side) fn move_remote_interaction(
        &mut self,
        id: i32,
        position: [f64; 3],
    ) {
        if let Some(i) = self.client.interactions.get_mut(&id) {
            i.position = Vec3::new(position[0], position[1], position[2]);
            self.pick_section_move(id as u32, position);
        }
    }
    pub(in crate::game::client_side) fn remote_interaction_data(
        &mut self,
        id: i32,
        values: &[(u8, i32, Value)],
    ) -> bool {
        let Some(i) = self.client.interactions.get_mut(&id) else {
            return false;
        };
        metadata(i, values);
        true
    }
}

pub(in crate::game) fn snapshot_previous(interaction: &mut RemoteInteraction) {
    interaction.previous = interaction.position;
    interaction.previous_yaw = interaction.yaw;
    interaction.previous_pitch = interaction.pitch;
}

pub(in crate::game) fn set_rot(interaction: &mut RemoteInteraction, yaw: f32, pitch: f32) {
    if yaw.is_finite() {
        interaction.yaw = yaw % 360.0;
    }
    if pitch.is_finite() {
        interaction.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
    }
}

pub(in crate::game) fn snap_to(
    interaction: &mut RemoteInteraction,
    position: Vec3,
    yaw: f32,
    pitch: f32,
) {
    interaction.position = position;
    if yaw.is_finite() {
        interaction.yaw = yaw;
    }
    if pitch.is_finite() {
        interaction.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
    }
    snapshot_previous(interaction);
}

/// `Entity.positionRider`'s move: a passenger is placed on its vehicle's
/// seat, not moved on its own.
pub(in crate::game) fn ride_to(interaction: &mut RemoteInteraction, at: glam::DVec3) {
    interaction.position = Vec3::new(at.x, at.y, at.z);
}

fn metadata(interaction: &mut RemoteInteraction, values: &[(u8, i32, Value)]) {
    for (index, _, value) in values {
        match (index, value) {
            (6, _) => interaction.dimensions_refreshed = true,
            (8, Value::Float(width)) => {
                interaction.width = *width;
                interaction.dimensions_refreshed = true;
            }
            (9, Value::Float(height)) => {
                interaction.height = *height;
                interaction.dimensions_refreshed = true;
            }
            (10, Value::Bool(response)) => interaction.response = *response,
            _ => {}
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn the_box_is_centred_on_the_feet_and_grows_with_the_data() {
        let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
        let b = i.bounds();
        assert_eq!(
            (b.min_position().components(), b.max_position().components()),
            ([9.5, 64.0, -3.5], [10.5, 65.0, -2.5])
        );
        metadata(
            &mut i,
            &[
                (8, 0, Value::Float(2.2)),
                (9, 0, Value::Float(2.2)),
                (10, 0, Value::Bool(true)),
            ],
        );
        let b = i.bounds();
        let half = f64::from(2.2f32) / 2.0;
        let height = f64::from(2.2f32);
        let (lo, hi) = (b.min_position().components(), b.max_position().components());
        for (got, want) in lo.iter().zip([10.0 - half, 64.0, -3.0 - half]) {
            assert!((got - want).abs() < 1e-6, "{lo:?}");
        }
        for (got, want) in hi.iter().zip([10.0 + half, 64.0 + height, -3.0 + half]) {
            assert!((got - want).abs() < 1e-6, "{hi:?}");
        }
        assert!(i.response);
    }
    #[test]
    fn a_riding_hitbox_moves_with_its_pivot() {
        let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
        i.vehicle = Some(9);
        ride_to(&mut i, glam::DVec3::new(12.0, 65.5, -3.0));
        assert_eq!(i.position, Vec3::new(12.0, 65.5, -3.0));
        let b = i.bounds();
        assert_eq!(
            (b.min_position().components(), b.max_position().components()),
            ([11.5, 65.5, -3.5], [12.5, 66.5, -2.5])
        );
    }

    fn parity_witness() -> serde_json::Value {
        let path = std::env::var("SUNRISE_CAMERA_PARITY_WITNESS")
            .expect("run through tools/test_camera_parity.py, which records the witness");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "failed to read SUNRISE_CAMERA_PARITY_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
            )
        });
        serde_json::from_str(&raw).expect("valid 26.3 camera parity witness")
    }

    fn hex_f64(v: &serde_json::Value) -> u64 {
        u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
    }

    #[test]
    #[ignore = "run by tools/test_camera_parity.py"]
    fn interaction_unsynced_default_sync_and_pose_sync_seat_height_26_3() {
        run_interaction_unsynced_default_sync_and_pose_sync_seat_height_26_3();
    }

    pub(in crate::game::client_side) fn run_interaction_unsynced_default_sync_and_pose_sync_seat_height_26_3()
     {
        let witness = parity_witness();
        let life = &witness["interaction_lifecycle"];

        let mut i = RemoteInteraction::new([0.5, 300.0, 0.5]);
        assert_eq!(
            i.seat_height().to_bits(),
            hex_f64(&life["unsynced_spawn"]["seat_height_bits"]),
            "unsynced interaction seat height"
        );

        // Explicit default (1.0, 1.0) triggers refreshDimensions() in vanilla 26.3.
        metadata(
            &mut i,
            &[(8, 3, Value::Float(1.0)), (9, 3, Value::Float(1.0))],
        );
        assert_eq!(
            i.seat_height().to_bits(),
            hex_f64(&life["sync_default_1x1"]["seat_height_bits"]),
            "explicit 1x1 sync must refresh seat height to 1.0"
        );

        // DATA_POSE (6) alone also triggers refreshDimensions() on an unsynced Interaction.
        let mut pose_only = RemoteInteraction::new([0.5, 300.0, 0.5]);
        metadata(&mut pose_only, &[(6, 21, Value::Id(0))]);
        assert_eq!(
            pose_only.seat_height().to_bits(),
            hex_f64(&life["sync_pose_only"]["seat_height_bits"]),
            "DATA_POSE sync must refresh seat height to 1.0"
        );
    }

    #[test]
    #[ignore = "run by tools/test_camera_parity.py"]
    fn interaction_negative_height_bounds_invert_downward_26_3() {
        run_interaction_negative_height_bounds_invert_downward_26_3();
    }

    pub(in crate::game::client_side) fn run_interaction_negative_height_bounds_invert_downward_26_3()
     {
        let witness = parity_witness();
        let life = &witness["interaction_lifecycle"];
        for (case_name, width, height) in [
            ("sync_negative_height", 0.5f32, -1.5f32),
            ("sync_negative_width_and_height", -0.4f32, -2.0f32),
        ] {
            let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
            metadata(
                &mut i,
                &[(8, 3, Value::Float(width)), (9, 3, Value::Float(height))],
            );
            let b = i.bounds();
            let got = [
                b.min_position().x.to_bits(),
                b.min_position().y.to_bits(),
                b.min_position().z.to_bits(),
                b.max_position().x.to_bits(),
                b.max_position().y.to_bits(),
                b.max_position().z.to_bits(),
            ];
            let want: Vec<u64> = life[case_name]["aabb_bits"]
                .as_array()
                .unwrap()
                .iter()
                .map(hex_f64)
                .collect();
            assert_eq!(got.as_slice(), want.as_slice(), "{case_name}");
        }
    }
}
