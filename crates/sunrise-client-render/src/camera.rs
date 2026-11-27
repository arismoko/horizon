//! Camera projection, frustum near-plane, and entity alignment (`view_matrix` and `NearPlane`
//! remain here for `internals.txt` and `#[replaceable]`).
use joml::Quat;
use sunrise_core::geometry::trig;
pub use sunrise_level::camera_math::*;

const DEG_TO_RAD: f32 = trig::DEG_TO_RAD;

/// Camera-relative world view used by terrain, sky and the first-person hand.
#[sunrise_mod_api_macros::replaceable]
pub fn view_matrix(yaw: f32, pitch: f32) -> glam::Mat4 {
    view_matrix_rolled(yaw, pitch, 0.0)
}

/// `GameRenderer.projectPointToScreen`: world point `at` seen from a camera at `position` facing
/// `yaw`/`pitch` through column-major `projection` (the frame's reversed-Z perspective), as
/// normalised device x and y (-1 to 1 across the screen) and depth z. The view is the camera's
/// rotation only, without bob or spin, as vanilla's. None for a point at or behind the camera and
/// for a non-finite `at`, where vanilla returns a mirrored or NaN point.
pub fn project_point(
    projection: &[f32; 16],
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    at: [f64; 3],
) -> Option<[f32; 3]> {
    project_point_rolled(projection, position, yaw, pitch, 0.0, at)
}

/// [`project_point`] with a roll about the view direction, in degrees.
pub fn project_point_rolled(
    projection: &[f32; 16],
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    roll: f32,
    at: [f64; 3],
) -> Option<[f32; 3]> {
    if !at.iter().all(|v| v.is_finite()) {
        return None;
    }
    // Vanilla subtracts in doubles, then transforms the offset as floats.
    let offset = glam::Vec3::new(
        (at[0] - position[0]) as f32,
        (at[1] - position[1]) as f32,
        (at[2] - position[2]) as f32,
    );
    let view = if roll == 0.0 {
        view_matrix(yaw, pitch)
    } else {
        view_matrix_rolled(yaw, pitch, roll)
    };
    let clip = glam::Mat4::from_cols_array(projection) * view * offset.extend(1.0);
    if clip.w.is_nan() || clip.w <= 0.0 {
        return None;
    }
    let ndc = [clip.x / clip.w, clip.y / clip.w, clip.z / clip.w];
    ndc.iter().all(|v| v.is_finite()).then_some(ndc)
}

/// `Camera.NearPlane`: the near plane's centre and its half extents toward
/// the camera's left and up, relative to the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearPlane {
    pub forward: [f64; 3],
    pub left: [f64; 3],
    pub up: [f64; 3],
}
impl NearPlane {
    /// `forward`, `getTopLeft`, `getTopRight`, `getBottomLeft`, `getBottomRight`:
    /// the points `Camera.getFluidInCamera` looks for lava and powder snow at.
    pub fn points(&self) -> [[f64; 3]; 5] {
        let corner = |up: f64, left: f64| -> [f64; 3] {
            std::array::from_fn(|i| self.forward[i] + self.up[i] * up + self.left[i] * left)
        };
        [
            self.forward,
            corner(1.0, 1.0),
            corner(1.0, -1.0),
            corner(-1.0, 1.0),
            corner(-1.0, -1.0),
        ]
    }
    /// `getPointOnPlane(x, y)`: `forward + up * y - left * x`.
    pub fn point_on_plane(&self, x: f32, y: f32) -> [f64; 3] {
        std::array::from_fn(|i| {
            self.forward[i] + self.up[i] * f64::from(y) - self.left[i] * f64::from(x)
        })
    }
}

/// `Camera`: the eye-height and field-of-view easing and the view placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub initialized: bool,
    pub eye_height: f32,
    pub eye_height_old: f32,
    pub fov_modifier: f32,
    pub old_fov_modifier: f32,
    pub position: [f64; 3],
    /// Where the camera was placed before a mod's `camera_position` overrides: streaming and
    /// the level-entry readiness follow this, never the render-only `position`.
    pub placed_position: [f64; 3],
    pub x_rot: f32,
    pub y_rot: f32,
    pub roll: f32,
    pub rotation: Quat,
    pub forwards: [f32; 3],
    pub up: [f32; 3],
    pub left: [f32; 3],
    pub detached: bool,
    /// The entity's eye and view rotation (yaw, pitch), where a detached camera backs off
    /// from.
    pub eye: [f64; 3],
    pub eye_rotation: [f32; 2],
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            initialized: false,
            eye_height: 0.0,
            eye_height_old: 0.0,
            fov_modifier: 0.0,
            old_fov_modifier: 0.0,
            position: [0.0; 3],
            placed_position: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            roll: 0.0,
            rotation: Quat::IDENTITY,
            forwards: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            left: [-1.0, 0.0, 0.0],
            detached: false,
            eye: [0.0; 3],
            eye_rotation: [0.0; 2],
        }
    }
}
impl Camera {
    /// `Camera.tick`: the eye height eases halfway toward the entity's
    /// (with a level and an entity), then `tickFov` with the camera entity's
    /// field-of-view modifier target (1 when it is not a player).
    pub fn tick(&mut self, entity_eye_height: Option<f32>, target_fov_modifier: f32) {
        if let Some(eye) = entity_eye_height {
            self.eye_height_old = self.eye_height;
            self.eye_height += (eye - self.eye_height) * 0.5;
        }
        self.old_fov_modifier = self.fov_modifier;
        self.fov_modifier += (target_fov_modifier - self.fov_modifier) * 0.5;
        self.fov_modifier = clamp(self.fov_modifier, 0.1, 1.5);
    }
    /// `Camera.setRotation`.
    pub fn set_rotation(&mut self, y_rot: f32, x_rot: f32) {
        self.set_rotation_with_roll(y_rot, x_rot, 0.0);
    }
    /// `Camera.setRotation` with NeoForge's `roll` (in degrees; `rotationYXZ(PI - yaw, -pitch, -roll)`).
    pub fn set_rotation_with_roll(&mut self, y_rot: f32, x_rot: f32, roll: f32) {
        self.x_rot = x_rot;
        self.y_rot = y_rot;
        self.roll = roll;
        self.rotation = rotation_with_roll(y_rot, x_rot, roll);
        self.forwards = self.rotation.transform([0.0, 0.0, -1.0]);
        self.up = self.rotation.transform([0.0, 1.0, 0.0]);
        self.left = self.rotation.transform([-1.0, 0.0, 0.0]);
    }
    /// `Camera.getNearPlane(fov)` for a `width` x `height` window, at the
    /// projection's `zNear` of 0.05.
    pub fn near_plane(&self, fov: f32, width: f32, height: f32) -> NearPlane {
        let z_near = f64::from(0.05f32);
        let aspect = f64::from(width) / f64::from(height);
        let plane_height = sunrise_core::jmath::tan(f64::from(fov * DEG_TO_RAD) / 2.0) * z_near;
        let scale = |v: [f32; 3], by: f64| v.map(|c| f64::from(c) * by);
        NearPlane {
            forward: scale(self.forwards, z_near),
            left: scale(self.left, plane_height * aspect),
            up: scale(self.up, plane_height),
        }
    }
    /// `Camera.move(forwards, up, right)`.
    pub fn move_by(&mut self, forwards: f32, up: f32, right: f32) {
        let offset = self.rotation.transform([right, up, -forwards]);
        self.position = [
            self.position[0] + f64::from(offset[0]),
            self.position[1] + f64::from(offset[1]),
            self.position[2] + f64::from(offset[2]),
        ];
    }
    /// The entity's interpolated eye and look, independent of the final render camera.
    pub fn align_eye(&mut self, entity: &CameraEntity, partial: f32) {
        let p = f64::from(partial);
        self.eye = [
            lerp_d(p, entity.previous[0], entity.position[0]),
            lerp_d(p, entity.previous[1], entity.position[1])
                + f64::from(lerp(partial, self.eye_height_old, self.eye_height)),
            lerp_d(p, entity.previous[2], entity.position[2]),
        ];
        self.eye_rotation = [entity.view_y_rot(partial), entity.view_x_rot(partial)];
    }

    /// `Camera.alignWithEntity` (not riding a minecart with the new
    /// movement): the entity's view rotation and interpolated eye, then a
    /// detached camera backed off along the view (`getMaxZoom`) or a sleeping
    /// one turned to the bed. `clip` is `Level.clip` with
    /// `ClipContext.Block.VISUAL` and no fluids; it returns the hit location
    /// unless the ray misses.
    pub fn align_with_entity(
        &mut self,
        entity: &CameraEntity,
        partial: f32,
        camera_type: CameraType,
        clip: impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) {
        self.align_with_entity_distance(entity, partial, camera_type, |distance| distance, clip);
    }
    /// Adjust the detached distance after entity/mount scaling, before collision clipping.
    /// Only finite overrides in 0..=1024 blocks are accepted (within the game's ray budget).
    /// First-person views do not call `adjust_distance`.
    pub fn align_with_entity_distance(
        &mut self,
        entity: &CameraEntity,
        partial: f32,
        camera_type: CameraType,
        adjust_distance: impl FnOnce(f32) -> f32,
        clip: impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) {
        self.align_with_entity_angles(
            entity,
            partial,
            camera_type,
            (entity.view_y_rot(partial), entity.view_x_rot(partial), 0.0),
            adjust_distance,
            clip,
        );
    }
    /// `Camera.alignWithEntity` with angles `(yaw, pitch, roll)` supplied at NeoForge's
    /// `ViewportEvent.ComputeCameraAngles` moment — before `setRotation`, third-person mirroring
    /// (`yaw + 180`, `-pitch`, `-roll`), detached back-off, and sleeping bed rotation.
    pub fn align_with_entity_angles(
        &mut self,
        entity: &CameraEntity,
        partial: f32,
        camera_type: CameraType,
        (yaw, pitch, roll): (f32, f32, f32),
        adjust_distance: impl FnOnce(f32) -> f32,
        mut clip: impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) {
        self.set_rotation_with_roll(yaw, pitch, roll);
        self.align_eye(entity, partial);
        self.position = self.eye;
        self.detached = !camera_type.is_first_person();
        if self.detached {
            if camera_type.is_mirrored() {
                self.set_rotation_with_roll(self.y_rot + 180.0, -self.x_rot, -self.roll);
            }
            let (mut distance, mut scale) = (4.0f32, 1.0f32);
            if let Some(living) = &entity.living {
                scale = living.scale;
                distance = living.camera_distance as f32;
            }
            let (mut mount_scale, mut mount_distance) = (scale, distance);
            if let Some((s, d)) = entity.mount {
                mount_scale = s;
                mount_distance = d as f32;
            }
            let vanilla = java_max(scale * distance, mount_scale * mount_distance);
            let adjusted = adjust_distance(vanilla);
            let distance = if adjusted.is_finite() && (0.0..=1024.0).contains(&adjusted) {
                adjusted
            } else {
                vanilla
            };
            let zoom = self.max_zoom(distance, &mut clip);
            self.move_by(-zoom, 0.0, 0.0);
        } else if let Some(LivingCamera {
            sleeping: Some(bed),
            ..
        }) = entity.living
        {
            self.set_rotation(bed.map_or(0.0, |yaw| yaw - 180.0), 0.0);
            self.move_by(0.0, 0.15, 0.0);
        }
        self.initialized = true;
    }
    /// `Camera.getMaxZoom`: eight rays from the corners of a 0.2 cube about
    /// the eye, shortening the distance to the nearest visual-shape hit.
    pub fn max_zoom(
        &self,
        mut distance: f32,
        clip: &mut impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) -> f32 {
        let at = self.position;
        for i in 0..8 {
            let offset = [
                ((i & 1) * 2 - 1) as f32,
                ((i >> 1 & 1) * 2 - 1) as f32,
                ((i >> 2 & 1) * 2 - 1) as f32,
            ];
            let from: [f64; 3] = std::array::from_fn(|a| at[a] + f64::from(offset[a] * 0.1));
            let back = f64::from(-distance);
            let to: [f64; 3] =
                std::array::from_fn(|a| from[a] + f64::from(self.forwards[a]) * back);
            if let Some(hit) = clip(from, to) {
                let d = [hit[0] - at[0], hit[1] - at[1], hit[2] - at[2]];
                let dist_sq = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as f32;
                if dist_sq < distance * distance {
                    distance = dist_sq.sqrt();
                }
            }
        }
        distance
    }
    /// `Camera.calculateFov`: the option scaled by the eased modifier, then
    /// the death and fluid effects.
    pub fn calculate_fov(
        &self,
        partial: f32,
        fov_option: i32,
        effect_scale: f32,
        dead: Option<i32>,
        fluid: CameraFluid,
        panoramic: bool,
    ) -> f32 {
        if panoramic {
            return 90.0;
        }
        let fov = fov_option as f32 * lerp(partial, self.old_fov_modifier, self.fov_modifier);
        modify_fov_based_on_death_or_fluid(partial, fov, effect_scale, dead, fluid)
    }
    /// `Camera.calculateHudFov`.
    pub fn calculate_hud_fov(
        &self,
        partial: f32,
        effect_scale: f32,
        dead: Option<i32>,
        fluid: CameraFluid,
    ) -> f32 {
        modify_fov_based_on_death_or_fluid(partial, 70.0, effect_scale, dead, fluid)
    }
}

/// `Frustum.getNearPlaneBounds`: the box around the four corners of the
/// culling frustum's NDC `z = -1` plane, relative to the camera (the caller
/// moves it to the eye). The culling projection is `Camera.update`'s
/// `setPerspective(fov, aspect, 0.05, depthFar, zZeroToOne)`.
pub fn near_plane_bounds(
    fov_degrees: f32,
    width: f32,
    height: f32,
    yaw: f32,
    pitch: f32,
    far: f32,
) -> ([f64; 3], [f64; 3]) {
    let projection =
        glam::Mat4::from_cols_array(&perspective(fov_degrees, width, height, 0.05, far, true));
    // Frustum.calculateFrustum: projection * modelView.
    let clip_to_world = (projection * view_matrix(yaw, pitch)).inverse();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let p = clip_to_world * glam::Vec4::new(x, y, -1.0, 1.0);
        let p = [p.x / p.w, p.y / p.w, p.z / p.w];
        for k in 0..3 {
            min[k] = min[k].min(f64::from(p[k]));
            max[k] = max[k].max(f64::from(p[k]));
        }
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    /// A synthetic camera at (0, 64, 0) facing south (yaw 0, +Z), 70° FOV, 16:9, near 0.05, far
    /// 512, reversed Z as the game builds it.
    fn project(at: [f64; 3]) -> Option<[f32; 3]> {
        let projection =
            glam::Mat4::perspective_rh(70f32.to_radians(), 16.0 / 9.0, 512.0, 0.05).to_cols_array();
        super::project_point(&projection, [0.0, 64.0, 0.0], 0.0, 0.0, at)
    }

    #[test]
    fn projected_points_ahead_land_on_screen_and_behind_or_nan_are_none() {
        let ahead = project([0.0, 64.0, 10.0]).unwrap();
        assert!(ahead[0].abs() < 1e-6 && ahead[1].abs() < 1e-6, "{ahead:?}");
        // Reversed Z: depth near / distance, 0.05 / 10.
        assert!((ahead[2] - 0.005).abs() < 1e-4, "{ahead:?}");
        // Up and to the camera's right (west, -X, when facing south) at the edge of the view.
        let corner = (35f64.to_radians()).tan() * 10.0;
        let edge = project([-corner * 16.0 / 9.0, 64.0 + corner, 10.0]).unwrap();
        assert!(
            (edge[0] - 1.0).abs() < 1e-4 && (edge[1] - 1.0).abs() < 1e-4,
            "{edge:?}"
        );
        let near = project([0.0, 64.0, 0.05]).unwrap();
        assert!((near[2] - 1.0).abs() < 1e-5, "{near:?}");
        assert_eq!(project([f64::MAX, 64.0, 10.0]), None);
        assert_eq!(project([0.0, 64.0, -10.0]), None);
        assert_eq!(project([0.0, 64.0, 0.0]), None);
        assert_eq!(project([f64::NAN, 64.0, 10.0]), None);
        assert_eq!(project([0.0, f64::INFINITY, 10.0]), None);
    }
    #[test]
    fn near_plane_bounds_sit_half_a_near_distance_ahead_of_the_eye() {
        // Facing +Z (yaw 0): the zero-to-one culling projection puts NDC
        // z = -1 at -far * near / (2 far - near) in front of the camera.
        let far = 512.0;
        let (min, max) = super::near_plane_bounds(70.0, 1600.0, 900.0, 0.0, 0.0, far);
        let depth = f64::from(far * 0.05 / (2.0 * far - 0.05));
        assert!((min[2] - depth).abs() < 1e-4 && (max[2] - depth).abs() < 1e-4);
        let half_height = depth * 35.0f64.to_radians().tan();
        assert!((max[1] - half_height).abs() < 1e-4 && (min[1] + half_height).abs() < 1e-4);
        let half_width = half_height * 1600.0 / 900.0;
        assert!((max[0] - half_width).abs() < 1e-3 && (min[0] + half_width).abs() < 1e-3);
    }
    use super::*;
    #[test]
    fn view_keeps_yaw_at_vertical_pitch_limits() {
        for yaw in [0.0, 45.0, 90.0, 180.0, -90.0, 360.0] {
            for pitch in [-90.0, -89.999, 0.0, 89.999, 90.0] {
                let view = view_matrix(yaw, pitch);
                let mut camera = Camera::default();
                camera.set_rotation(yaw, pitch);
                for (world, expected) in [
                    (camera.forwards, -glam::Vec3::Z),
                    (camera.up, glam::Vec3::Y),
                    (camera.left, -glam::Vec3::X),
                ] {
                    let got = view.transform_vector3(glam::Vec3::from(world));
                    assert!(
                        (got - expected).length() < 1e-5,
                        "yaw={yaw}, pitch={pitch}: {got:?} != {expected:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn camera_eases_eye_height_and_fov() {
        let mut camera = Camera::default();
        camera.tick(Some(1.62), 1.0);
        assert_eq!(camera.eye_height, 0.81);
        assert_eq!(camera.fov_modifier, 0.5);
        camera.tick(None, 1.0);
        assert_eq!(camera.eye_height, 0.81);
        assert_eq!(camera.fov_modifier, 0.75);
    }
    #[test]
    fn rotation_faces_south_at_zero_yaw() {
        let mut camera = Camera::default();
        camera.set_rotation(0.0, 0.0);
        assert!((camera.forwards[2] - 1.0).abs() < 1e-6);
        camera.set_rotation(90.0, 0.0);
        assert!((camera.forwards[0] + 1.0).abs() < 1e-6);
    }
    #[test]
    fn detached_camera_backs_off_until_blocked() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: None,
            mount: None,
        };
        let mut camera = Camera::default();
        camera.align_with_entity(&entity, 1.0, CameraType::ThirdPersonBack, |_, _| None);
        assert!((camera.position[2] + 4.0).abs() < 1e-6);
        camera.align_with_entity(&entity, 1.0, CameraType::ThirdPersonBack, |_, _| {
            Some([0.0, 0.0, -1.0])
        });
        assert!(camera.position[2] > -1.1 && camera.position[2] < -0.9);
    }
    #[test]
    fn detached_distance_override_keeps_collision_clipping() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: Some(LivingCamera {
                scale: 2.0,
                camera_distance: 3.0,
                sleeping: None,
                dead: None,
            }),
            mount: Some((3.0, 4.0)),
        };
        for kind in [CameraType::ThirdPersonBack, CameraType::ThirdPersonFront] {
            let mut camera = Camera::default();
            let mut calls = 0;
            let mut rays = 0;
            let z = if kind.is_mirrored() { 1.0 } else { -1.0 };
            camera.align_with_entity_distance(
                &entity,
                1.0,
                kind,
                |distance| {
                    assert_eq!(distance, 12.0);
                    8.0
                },
                |_, _| None,
            );
            assert!((camera.position[2] - z * 8.0).abs() < 1e-5);
            camera.align_with_entity_distance(
                &entity,
                1.0,
                kind,
                |distance| {
                    calls += 1;
                    assert_eq!(distance, 12.0);
                    8.0
                },
                |_, _| {
                    rays += 1;
                    Some([0.0, 0.0, z])
                },
            );
            assert_eq!((calls, rays), (1, 8));
            assert!((camera.position[2] - z).abs() < 0.2);
        }
    }
    #[test]
    fn detached_distance_rejects_invalid_values_and_accepts_zero() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: None,
            mount: None,
        };
        for distance in [f32::NAN, f32::INFINITY, -1.0, 1025.0] {
            let mut camera = Camera::default();
            camera.align_with_entity_distance(
                &entity,
                1.0,
                CameraType::ThirdPersonBack,
                |_| distance,
                |_, _| None,
            );
            assert!((camera.position[2] + 4.0).abs() < 1e-6);
        }
        let mut camera = Camera::default();
        camera.align_with_entity_distance(
            &entity,
            1.0,
            CameraType::ThirdPersonBack,
            |_| 0.0,
            |_, _| None,
        );
        assert_eq!(camera.position, camera.eye);
        camera.align_with_entity_distance(
            &entity,
            1.0,
            CameraType::FirstPerson,
            |_| panic!("first person has no detached distance"),
            |_, _| None,
        );
        assert_eq!(camera.position, camera.eye);
    }
    #[test]
    fn align_with_entity_angles_mirrors_roll_in_third_person_front_and_backs_off_along_new_yaw() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: None,
            mount: None,
        };
        let mut camera = Camera::default();
        camera.align_with_entity_angles(
            &entity,
            1.0,
            CameraType::ThirdPersonBack,
            (90.0, 0.0, 15.0),
            |d| d,
            |_, _| None,
        );
        assert_eq!((camera.y_rot, camera.x_rot, camera.roll), (90.0, 0.0, 15.0));
        // Yaw 90 faces -X, so backing off moves +4 along +X.
        assert!((camera.position[0] - 4.0).abs() < 1e-5);
        camera.align_with_entity_angles(
            &entity,
            1.0,
            CameraType::ThirdPersonFront,
            (90.0, 20.0, 15.0),
            |d| d,
            |_, _| None,
        );
        assert_eq!(
            (camera.y_rot, camera.x_rot, camera.roll),
            (270.0, -20.0, -15.0)
        );
        let view = view_matrix_rolled(camera.y_rot, camera.x_rot, camera.roll);
        for (world, expected) in [
            (camera.forwards, -glam::Vec3::Z),
            (camera.up, glam::Vec3::Y),
            (camera.left, -glam::Vec3::X),
        ] {
            let got = view.transform_vector3(glam::Vec3::from(world));
            assert!((got - expected).length() < 1e-5, "{got:?} != {expected:?}");
        }
    }
    #[test]
    fn fov_modifier_combines_speed_flight_and_bow() {
        let mut p = FovPlayer {
            flying: false,
            walking_speed: 0.1,
            movement_speed: 0.13,
            using: None,
        };
        assert_eq!(
            field_of_view_modifier(&p, true, 1.0),
            (0.13f32 / 0.1 + 1.0) / 2.0
        );
        assert_eq!(field_of_view_modifier(&p, true, 0.0), 1.0);
        p.using = Some((UseItem::Spyglass, 3));
        assert_eq!(field_of_view_modifier(&p, true, 1.0), 0.1);
        assert_ne!(field_of_view_modifier(&p, false, 1.0), 0.1);
    }
}
