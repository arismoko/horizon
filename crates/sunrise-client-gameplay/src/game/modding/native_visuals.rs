use crate::game::ClientGame;
use sunrise_client::{dynamic_light, gizmos};

/// The lights the mods asked for this frame, kept until the nearest are selected.
#[derive(Default)]
pub struct FrameVisuals {
    mod_lights: Vec<mod_api::ColoredLight>,
}

impl FrameVisuals {
    pub fn finish(self, camera: glam::DVec3) -> Vec<dynamic_light::Colored> {
        dynamic_light::select_colored(
            self.mod_lights
                .into_iter()
                .map(|l| (l.at, l.rgb, l.radius, l.intensity)),
            camera,
        )
    }
}

pub fn rim_target_valid(
    loaded: bool,
    state: sunrise_world::registry::BlockStateId,
    air: sunrise_world::registry::BlockStateId,
) -> bool {
    loaded && state != air
}

impl ClientGame<'_> {
    /// Frame-local requests never persist after a mod is removed or a world changes.
    pub fn native_frame_visuals(&self, lines: &mut Vec<gizmos::Line>) -> FrameVisuals {
        let mut visuals = FrameVisuals::default();
        let Some(position) = self.common.player.as_ref().map(|player| player.position) else {
            return visuals;
        };
        let mut rims = Vec::new();
        let mut lights = Vec::new();
        self.native_visuals(&mut rims, &mut lights);
        let close = |at: [f64; 3]| {
            at.iter().all(|v| v.is_finite())
                && (at[0] - position.x).powi(2)
                    + (at[1] - position.y).powi(2)
                    + (at[2] - position.z).powi(2)
                    <= 1024.0
        };
        let mut edges = 0;
        for rim in rims.into_iter().take(8) {
            let pos = rim.pos;
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            if edges >= 256
                || !rim.width.is_finite()
                || !(0.5..=4.0).contains(&rim.width)
                || !close(pos.map(|v| f64::from(v) + 0.5))
                || !rim_target_valid(
                    self.common
                        .world
                        .chunks
                        .contains_key(&(pos[0].div_euclid(16), pos[2].div_euclid(16))),
                    state,
                    self.common.world.air,
                )
            {
                continue;
            }
            for edge in self.outline_edges_iter(state, pos).take(256 - edges) {
                lines.push(gizmos::Line {
                    start: [edge[0], edge[1], edge[2]],
                    end: [edge[3], edge[4], edge[5]],
                    color: rim.argb,
                    width: rim.width,
                });
                edges += 1;
            }
        }
        lights.truncate(8);
        lights.retain(|l| {
            close(l.at)
                && l.rgb <= 0xffffff
                && l.radius.is_finite()
                && (0.25..=16.0).contains(&l.radius)
                && l.intensity.is_finite()
                && (0.0..=1.0).contains(&l.intensity)
                && self.common.world.chunks.contains_key(&(
                    (l.at[0].floor() as i32).div_euclid(16),
                    (l.at[2].floor() as i32).div_euclid(16),
                ))
        });
        visuals.mod_lights = lights;
        visuals
    }
}
