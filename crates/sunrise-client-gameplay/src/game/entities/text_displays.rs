//! Server text displays: metadata, interpolation, billboard transforms and world text.
use crate::game::ClientGame;
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    gfx::{
        TEXT_POLYGON_OFFSET_OVERLAY, TEXT_POLYGON_OFFSET_SOLID_OVERLAY, TRANSLUCENT_DRAW_FLIP,
        TRANSLUCENT_GLINT_ITEM_OVERLAY, TRANSLUCENT_ITEM_OVERLAY,
    },
    mesher::Vertex,
    text_display::{self, CachedQuads, TextDisplayData},
};
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_protocol::entity_data::Value;
use sunrise_world::registry::BlockStateId;

/// What a display shows besides its transformation: its text (in `data`),
/// a block state (`BlockDisplay`) or an item and how it is displayed
/// (`ItemDisplay`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shows {
    Text,
    Block,
    Item,
}

impl Shows {
    pub(in crate::game) fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "minecraft:text_display" => Some(Self::Text),
            "minecraft:block_display" => Some(Self::Block),
            "minecraft:item_display" => Some(Self::Item),
            _ => None,
        }
    }

    /// The display's entity type, as the tick hooks name it.
    pub(in crate::game::client_side) fn kind(self) -> &'static str {
        match self {
            Self::Text => "minecraft:text_display",
            Self::Block => "minecraft:block_display",
            Self::Item => "minecraft:item_display",
        }
    }
}

pub struct RemoteTextDisplay {
    pub data: TextDisplayData,
    pub(in crate::game) join: sunrise_sim::join_seq::JoinSeq,
    pub(in crate::game) shows: Shows,
    /// Pending synced sub-state fields (`BlockDisplay` id 23, `ItemDisplay` ids 23-24).
    pub(in crate::game) pending_block: Option<BlockStateId>,
    pub(in crate::game) pending_item: Option<ItemStack>,
    pub(in crate::game) pending_item_display: u8,
    /// Committed sub-state updated inside `tick()` only when `dirty` (`updateRenderSubState`).
    pub(in crate::game) shown_block: Option<BlockStateId>,
    pub(in crate::game) shown_item: Option<ItemStack>,
    pub(in crate::game) shown_item_display: u8,
    pub from: TextDisplayData,
    pub shown: TextDisplayData,
    /// True when the committed render state uses interpolated (`from -> shown`) rather than constant (`shown`).
    pub(in crate::game) interpolated: bool,
    pub position: glam::DVec3,
    pub previous: glam::DVec3,
    pub(in crate::game) target: glam::DVec3,
    pub(in crate::game) last_pos: glam::DVec3,
    pub rotation: [f32; 2],
    pub old_rotation: [f32; 2],
    pub(in crate::game) target_rotation: [f32; 2],
    pub(in crate::game) last_rot: [f32; 2],
    /// `Display.cullingBoundingBox` and `Entity.noCulling`, updated by `updateCulling()`.
    pub(in crate::game) culling_box: (glam::DVec3, glam::DVec3),
    pub(in crate::game) no_culling: bool,
    /// `LinearInterpolationHandler` length (raw client synced id 10, unclamped) and steps left.
    pub(in crate::game) interp_length: i32,
    pub(in crate::game) steps: i32,
    /// `Entity.tickCount`: wraps as Java `int` (`i32`) before widening to `i64` in `start`.
    pub(in crate::game) age: i32,
    pub start: i64,
    pub(in crate::game) duration: i32,
    pub(in crate::game) dirty: bool,
    pub(in crate::game) start_dirty: bool,
    pub(in crate::game) duration_dirty: bool,
    /// `Display.renderState != null`: false until the first tick after a render-state sync.
    pub(in crate::game) render_state_ready: bool,
    /// `Display.lastProgress`: the progress the last drawn frame evaluated (untouched when culled
    /// or while `duration <= 0`).
    pub last_progress: f32,
    /// `Entity.DATA_SHARED_FLAGS_ID` (id 0): bit 0 = on fire, bit 1 = shift/discrete,
    /// bit 5 = invisible, bit 6 = glowing.
    pub(in crate::game) shared_flags: u8,
    /// `Entity.DATA_CUSTOM_NAME` (id 2) and `DATA_CUSTOM_NAME_VISIBLE` (id 3).
    pub(in crate::game) custom_name: Option<Text>,
    pub(in crate::game) custom_name_visible: bool,
    /// Local text geometry from the last visited frame; world poses do not
    /// invalidate `TextDisplay.clientDisplayCache` or the glyph vertices.
    pub(in crate::game) lines: Option<CachedQuads>,
    /// `Entity.vehicle`: the mob it rides (`SetPassengers`), which places it
    /// every tick (`positionRider`), as NPC plugins hang a name over a head.
    pub(in crate::game) vehicle: Option<u32>,
}

/// Reusable frame scratch for sorting all `seeThrough` text submits (name tags, markers,
/// and see-through text displays) together in one `TranslucentFeatureRenderPhase`.
#[derive(Default)]
pub struct SeeThroughCollector {
    pub(in crate::game) verts: Vec<Vertex>,
    pub(in crate::game) submits: Vec<(f32, usize, std::ops::Range<usize>)>,
}

impl SeeThroughCollector {
    pub fn clear(&mut self) {
        self.verts.clear();
        self.submits.clear();
    }
    pub(in crate::game::client_side) fn buffer_mut(&mut self) -> &mut Vec<Vertex> {
        &mut self.verts
    }
    pub(in crate::game::client_side) fn start(&self) -> usize {
        self.verts.len()
    }
    pub(in crate::game::client_side) fn finish(&mut self, start: usize, dist_sq: f32) {
        if self.verts.len() > start {
            let order = self.submits.len();
            self.submits.push((dist_sq, order, start..self.verts.len()));
        }
    }
    pub fn flush(&mut self, out: &mut Vec<Vertex>) {
        if self.submits.is_empty() {
            return;
        }
        // TranslucentFeatureRenderPhase: Floats.compare(d2, d1), stable on ties.
        self.submits
            .sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, _, range) in self.submits.drain(..) {
            out.extend_from_slice(&self.verts[range]);
        }
        self.verts.clear();
    }
}

/// `TranslucentSubmit.computeDistanceToCameraSq` (`Mth.lengthSquared(m30, m31, m32)`).
pub(in crate::game::client_side) fn submit_distance_sq(x: f32, y: f32, z: f32) -> f32 {
    x * x + y * y + z * z
}

/// Reusable frame scratch for `translucentBlocksAndItems` (`TranslucentFeatureRenderPhase`),
/// sorting translucent block and item display submits far-to-near and marking feature-type
/// boundaries with `TRANSLUCENT_DRAW_FLIP` so classic upload sorting never mixes draws.
#[derive(Default)]
pub(in crate::game) struct TranslucentDisplayCollector {
    pub(in crate::game) verts: Vec<Vertex>,
    pub(in crate::game) submits: Vec<(f32, usize, u8, std::ops::Range<usize>)>,
    pub(in crate::game) scratch: Vec<Vertex>,
}

impl TranslucentDisplayCollector {
    pub(in crate::game::client_side) fn clear(&mut self) {
        self.verts.clear();
        self.submits.clear();
        self.scratch.clear();
    }
    pub(in crate::game::client_side) fn push(
        &mut self,
        dist_sq: f32,
        feature: u8,
        range: std::ops::Range<usize>,
    ) {
        if !range.is_empty() {
            let order = self.submits.len();
            self.submits.push((dist_sq, order, feature, range));
        }
    }
    pub(in crate::game::client_side) fn flush(&mut self, out: &mut Vec<Vertex>) {
        if self.submits.is_empty() {
            return;
        }
        self.submits
            .sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut flip = 0u32;
        let mut prev_feature: Option<u8> = None;
        for (_, _, feature, range) in self.submits.drain(..) {
            if let Some(prev) = prev_feature
                && prev != feature
            {
                flip ^= TRANSLUCENT_DRAW_FLIP;
            }
            prev_feature = Some(feature);
            out.extend(self.verts[range].iter().map(|v| Vertex {
                layer: v.layer | flip,
                ..*v
            }));
        }
        self.verts.clear();
    }
}

/// `Display.RENDER_STATE_IDS` and each subclass's render-state synced data IDs (not view range,
/// width, height, or glow colour).
pub(in crate::game::client_side) fn renders(shows: Shows, id: u8) -> bool {
    let shared = match id {
        11..=16 | 18 | 19 => true,
        23..=27 => shows == Shows::Text,
        _ => false,
    };
    shared
        || match shows {
            Shows::Block => id == 23,
            Shows::Item => matches!(id, 23 | 24),
            Shows::Text => false,
        }
}

/// `Mth.lerp(double, double, double)` applied per axis (`Vec3.lerp`), avoiding `glam::DVec3::lerp`'s
/// `a * (1 - t) + b * t` 1-ULP drift.
pub(in crate::game::client_side) fn lerp_dvec3(
    start: glam::DVec3,
    end: glam::DVec3,
    delta: f64,
) -> glam::DVec3 {
    glam::DVec3::new(
        start.x + delta * (end.x - start.x),
        start.y + delta * (end.y - start.y),
        start.z + delta * (end.z - start.z),
    )
}

/// `Mth.wrapDegrees(float)`.
pub(in crate::game::client_side) fn wrap_degrees(value: f32) -> f32 {
    let mut f = value % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

/// `Mth.wrapDegrees(double)`.
pub(in crate::game::client_side) fn wrap_degrees_f64(value: f64) -> f64 {
    let mut d = value % 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// `Mth.rotLerp(double, double, double)`.
pub(in crate::game::client_side) fn rot_lerp_f64(delta: f64, start: f64, end: f64) -> f64 {
    start + delta * wrap_degrees_f64(end - start)
}

/// `Mth.rotLerp(float, float, float)`.
pub(in crate::game::client_side) fn rot_lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * wrap_degrees(end - start)
}

/// `Math.clamp(value, -90.0F, 90.0F)` matching `PositionMoveRotation.calculateAbsolute` / `Entity.setXRot`.
pub(in crate::game::client_side) fn clamp_pitch(pitch: f32) -> f32 {
    if pitch.is_nan() {
        f32::NAN
    } else {
        pitch.clamp(-90.0, 90.0)
    }
}

impl RemoteTextDisplay {
    pub fn new(shows: Shows, position: [f64; 3], yaw: f32) -> Self {
        let pos = glam::DVec3::from(position);
        let mut display = Self {
            data: Default::default(),
            join: sunrise_sim::join_seq::JoinSeq::next(),
            shows,
            pending_block: None,
            pending_item: None,
            pending_item_display: 0,
            shown_block: None,
            shown_item: None,
            shown_item_display: 0,
            from: Default::default(),
            shown: Default::default(),
            interpolated: false,
            position: pos,
            previous: pos,
            target: pos,
            last_pos: pos,
            rotation: [0.0, 0.0],
            old_rotation: [0.0, 0.0],
            target_rotation: [0.0, 0.0],
            last_rot: [0.0, 0.0],
            culling_box: (pos, pos),
            no_culling: true,
            interp_length: 0,
            steps: 0,
            age: 0,
            // Display.interpolationStartClientTick starts at Integer.MIN_VALUE so duration > 0
            // without a start-tick update snaps to progress 1.0.
            start: i64::from(i32::MIN),
            duration: 0,
            dirty: false,
            start_dirty: false,
            duration_dirty: false,
            render_state_ready: false,
            last_progress: 0.0,
            shared_flags: 0,
            custom_name: None,
            custom_name_visible: false,
            lines: None,
            vehicle: None,
        };
        display.set_y_rot_raw(yaw);
        display.old_rotation = display.rotation;
        display.target_rotation = display.rotation;
        display.last_rot = display.rotation;
        display.update_culling();
        display
    }
    /// Sets the initial spawn pitch (`Entity.snapTo` / `recreateFromPacket`).
    pub(in crate::game::client_side) fn with_pitch(mut self, pitch: f32) -> Self {
        self.set_x_rot_raw(pitch);
        self.old_rotation[1] = self.rotation[1];
        self.target_rotation[1] = self.rotation[1];
        self.last_rot[1] = self.rotation[1];
        self
    }
    /// `Entity.setYRot(float)`: stores raw finite yaw without `% 360.0`; discards non-finite.
    pub fn set_y_rot_raw(&mut self, yaw: f32) {
        if yaw.is_finite() {
            self.rotation[0] = yaw;
        }
    }
    /// `Entity.setXRot(float)`: reduces `% 360.0` before clamping to `[-90, 90]`; discards non-finite.
    pub fn set_x_rot_raw(&mut self, pitch: f32) {
        if pitch.is_finite() {
            self.rotation[1] = (pitch % 360.0).clamp(-90.0, 90.0);
        }
    }
    /// `Entity.setRot(float, float)`: reduces both yaw and pitch `% 360.0` before setters.
    pub(in crate::game::client_side) fn set_rot(&mut self, yaw: f32, pitch: f32) {
        self.set_y_rot_raw(yaw % 360.0);
        self.set_x_rot_raw(pitch % 360.0);
    }
    /// `Display.updateCulling`: refreshes `no_culling` and `culling_box` from current position, width and height.
    pub(in crate::game::client_side) fn update_culling(&mut self) {
        let (w, h) = (self.data.width, self.data.height);
        self.no_culling = text_display::no_culling(w, h);
        self.culling_box = if self.no_culling {
            (self.position, self.position)
        } else {
            text_display::culling_box(self.position, w, h)
        };
    }
    /// `Entity.setPos(Vec3)` -> `setPosRaw` + `updateCulling`.
    pub(in crate::game::client_side) fn set_pos(&mut self, pos: glam::DVec3) {
        self.position = pos;
        self.update_culling();
    }
    /// `Entity.snapTo(double, double, double, float, float)`.
    pub fn snap_to(&mut self, pos: glam::DVec3, yaw: f32, pitch: f32) {
        self.position = pos;
        self.set_y_rot_raw(yaw);
        self.set_x_rot_raw(pitch);
        self.previous = self.position;
        self.old_rotation = self.rotation;
        self.update_culling();
    }
    /// `AbstractInterpolationHandler.hasActiveInterpolation`.
    pub(in crate::game::client_side) fn is_interpolating(&self) -> bool {
        self.steps > 0
    }
    pub(in crate::game::client_side) fn cancel_interpolation(&mut self) {
        self.steps = 0;
    }
    /// `Entity.getClientPositionAndRotation`.
    pub(in crate::game::client_side) fn client_pos_rot(&self) -> (glam::DVec3, [f32; 2]) {
        if self.is_interpolating() {
            (self.target, self.target_rotation)
        } else {
            (self.position, self.rotation)
        }
    }
    /// `Entity.getYRot(float)` and `Entity.getXRot(float)`: exact stored value at `partial == 1.0`.
    pub fn rotation_at(&self, partial: f32) -> [f32; 2] {
        if partial == 1.0 {
            self.rotation
        } else {
            [
                rot_lerp(partial, self.old_rotation[0], self.rotation[0]),
                self.old_rotation[1] + partial * (self.rotation[1] - self.old_rotation[1]),
            ]
        }
    }
    /// `Display.getInterpolatedBoundingBox(float)`.
    pub fn interpolated_bounding_box(&self, partial: f32) -> (glam::DVec3, glam::DVec3) {
        if self.previous == self.position {
            self.culling_box
        } else {
            let delta =
                lerp_dvec3(self.previous, self.position, f64::from(partial)) - self.position;
            (self.culling_box.0 + delta, self.culling_box.1 + delta)
        }
    }
    /// `Camera.alignWithEntity` snapshot when the camera follows this display (`SetCamera`).
    pub(in crate::game::client_side) fn camera_entity(
        &self,
    ) -> sunrise_level::camera_math::CameraEntity {
        sunrise_level::camera_math::CameraEntity {
            position: self.position.to_array(),
            previous: self.previous.to_array(),
            x_rot: self.rotation[1],
            y_rot: self.rotation[0],
            x_rot_o: self.old_rotation[1],
            y_rot_o: self.old_rotation[0],
            eye_height: 0.0,
            living: None,
            mount: None,
        }
    }
    /// Where a passenger of this display sits, this tick and the last
    /// (`Display`'s box is 0 by 0, so its passenger attachment is its feet).
    pub(in crate::game::client_side) fn seat(&self) -> (glam::DVec3, glam::DVec3) {
        (self.position, self.previous)
    }
    /// `Entity.positionRider` through `setPos`: seats the display without cancelling active interpolation.
    pub fn ride(&mut self, at: glam::DVec3) {
        self.set_pos(at);
    }
    pub(in crate::game) fn position(&self) -> glam::DVec3 {
        self.position
    }
    /// `Entity.xo`, `yo`, `zo`: where the display was when its tick began (`setOldPosAndRot`).
    pub fn previous_position(&self) -> glam::DVec3 {
        self.previous
    }
    /// `Display.shouldRenderAtSqrDistance`: strict `<`, uninterpolated position, live `view_range`.
    pub fn in_range(&self, camera: glam::DVec3, view_scale: f64) -> bool {
        let range = f64::from(self.data.view_range) * 64.0 * view_scale;
        (self.position - camera).length_squared() < range * range
    }
    pub(in crate::game::client_side) fn render_position(&self, partial: f64) -> glam::DVec3 {
        lerp_dvec3(self.previous, self.position, partial)
    }
    pub(in crate::game::client_side) fn has_render_state(&self) -> bool {
        self.render_state_ready
    }
    pub(in crate::game::client_side) fn custom_name(&self) -> Option<&Text> {
        self.custom_name.as_ref()
    }
    pub(in crate::game::client_side) fn custom_name_visible(&self) -> bool {
        self.custom_name_visible
    }
    pub(in crate::game::client_side) fn is_discrete(&self) -> bool {
        self.shared_flags & 0x02 != 0
    }
    pub(in crate::game::client_side) fn is_on_fire(&self) -> bool {
        self.shared_flags & 0x01 != 0
    }
    pub(in crate::game::client_side) fn is_glowing(&self) -> bool {
        self.shared_flags & 0x40 != 0
    }
    pub(in crate::game::client_side) fn kind(&self) -> &'static str {
        self.shows.kind()
    }
    /// `DisplayRenderer.getBlockLightLevel`/`getSkyLightLevel` as packed 8-bit `(sky << 4) | block`.
    pub(in crate::game::client_side) fn packed_light(
        &self,
        world: &sunrise_level::world::World,
        pos: glam::DVec3,
    ) -> u8 {
        if self.data.brightness == -1 {
            let mut world_light = world.light(
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            );
            if self.is_on_fire() {
                world_light |= 0x0F;
            }
            world_light
        } else {
            let sky = (self.data.brightness as u32 >> 20) & 15;
            let block = (self.data.brightness as u32 >> 4) & 15;
            ((sky << 4) | block) as u8
        }
    }
    pub fn eval_render_state(&self, progress: f32) -> TextDisplayData {
        if self.interpolated {
            self.shown.blend(&self.from, progress)
        } else {
            self.shown.clone()
        }
    }
    pub fn progress(&self, partial: f32) -> f32 {
        if self.duration <= 0 {
            1.0
        } else {
            let elapsed = i64::from(self.age) - self.start;
            ((elapsed as f32 + partial) / self.duration as f32).clamp(0.0, 1.0)
        }
    }
    /// `Display.calculateInterpolationProgress`: only updates `last_progress` when `duration > 0`.
    pub fn calculate_interpolation_progress(&mut self, partial: f32) -> f32 {
        if self.duration <= 0 {
            return 1.0;
        }
        let progress = self.progress(partial);
        self.last_progress = progress;
        progress
    }
    /// The casters this display adds to the shadow pass: its interpolated
    /// shadow radius and strength (`DisplayRenderer.getShadowRadius` and
    /// `DisplayRenderer.getShadowStrength`), suppressed before first tick or when invisible.
    pub fn shadow(&self, partial: f64) -> Option<(glam::DVec3, f32, f32)> {
        if !self.render_state_ready || self.shared_flags & 0x20 != 0 {
            return None;
        }
        let d = self.eval_render_state(self.progress(partial as f32));
        (d.shadow_radius > 0.0).then_some((
            lerp_dvec3(self.previous, self.position, partial),
            d.shadow_radius,
            d.shadow_strength,
        ))
    }
    /// The entries after `Display`'s: a block state, or an item with its
    /// display context (the stack comes converted by the caller). Changes defer to `tick()`.
    pub fn show(&mut self, values: &[(u8, i32, Value)], item: Option<Option<ItemStack>>) {
        for (id, _, value) in values {
            match (self.shows, id, value) {
                (Shows::Block, 23, Value::Id(state)) => {
                    self.pending_block = Some(BlockStateId(u32::try_from(*state).unwrap_or(0)));
                    self.dirty = true;
                }
                (Shows::Item, 24, Value::Byte(context)) => {
                    self.pending_item_display = *context as u8;
                    self.dirty = true;
                }
                _ => {}
            }
        }
        if let Some(item) = item
            && self.shows == Shows::Item
        {
            self.pending_item = item;
            self.dirty = true;
        }
    }
    pub fn metadata(&mut self, values: &[(u8, i32, Value)]) {
        let mut culling_dirty = false;
        for (id, _, value) in values {
            match (id, value) {
                (0, Value::Byte(v)) => self.shared_flags = *v as u8,
                (2, Value::OptText(Some(tag))) | (2, Value::Text(tag)) => {
                    self.custom_name = Some(sunrise_sim::entity_nbt::name_text(tag));
                }
                (2, Value::OptText(None)) => self.custom_name = None,
                (3, Value::Bool(v)) => self.custom_name_visible = *v,
                (8, _) => self.start_dirty = true,
                (9, _) => self.duration_dirty = true,
                (10, Value::Int(v)) => self.interp_length = *v,
                (20 | 21, _) => culling_dirty = true,
                (id, _) if renders(self.shows, *id) => self.dirty = true,
                _ => {}
            }
        }
        self.data.apply(values);
        if culling_dirty {
            self.update_culling();
        }
    }
    pub fn tick(&mut self) {
        self.age = self.age.wrapping_add(1);
        self.previous = self.position;
        self.old_rotation = self.rotation;
        if !self.is_interpolating() {
            self.cancel_interpolation();
        } else {
            // AbstractInterpolationHandler.adjustInterpolationTargetFromDeltas.
            let delta = self.position - self.last_pos;
            if delta.length_squared() > f64::from(1.0e-5_f32) {
                self.target += delta;
            }
            self.target_rotation[0] += self.rotation[0] - self.last_rot[0];
            self.target_rotation[1] += self.rotation[1] - self.last_rot[1];
            // LinearInterpolationHandler.doInterpolate.
            let alpha = 1.0 / f64::from(self.steps);
            let new_pos = lerp_dvec3(self.position, self.target, alpha);
            let new_y = rot_lerp_f64(
                alpha,
                f64::from(self.rotation[0]),
                f64::from(self.target_rotation[0]),
            ) as f32;
            let new_x = (f64::from(self.rotation[1])
                + alpha * (f64::from(self.target_rotation[1]) - f64::from(self.rotation[1])))
                as f32;
            self.set_pos(new_pos);
            self.set_rot(new_y, new_x);
            self.steps -= 1;
            self.last_pos = self.position;
            self.last_rot = self.rotation;
        }
        if self.start_dirty {
            self.start = i64::from(self.age.wrapping_add(self.data.interpolation_delay));
        }
        if self.duration_dirty {
            self.duration = self.data.interpolation_duration;
        }
        if self.dirty {
            // Display.tick: shouldInterpolate is `this.interpolationDuration != 0`, preserving
            // an interpolated state even when duration is negative.
            let should_interpolate = self.duration != 0;
            if should_interpolate && self.render_state_ready {
                self.from = self.eval_render_state(self.last_progress);
                self.interpolated = true;
            } else {
                self.from = self.data.clone();
                self.interpolated = false;
            }
            self.shown = self.data.clone();
            self.shown_block = self.pending_block;
            self.shown_item = self.pending_item.clone();
            self.shown_item_display = self.pending_item_display;
            self.render_state_ready = true;
        }
        self.dirty = false;
        self.start_dirty = false;
        self.duration_dirty = false;
    }
    /// `AbstractInterpolationHandler.interpolateTo` / `LinearInterpolationHandler.startInterpolating`:
    /// resolves omitted components against the active target (or current pose) before checking
    /// `interp_length == 0`, and leaves an active matching target untouched even when `interp_length < 0`.
    pub fn move_to_opt(&mut self, position: Option<[f64; 3]>, rotation: Option<(f32, f32)>) {
        let (cur_pos, cur_rot) = self.client_pos_rot();
        let next_pos = position.map(glam::DVec3::from).unwrap_or(cur_pos);
        let (next_y, next_x) = rotation.unwrap_or((cur_rot[0], cur_rot[1]));
        if self.interp_length == 0 {
            self.snap_to(next_pos, next_y, next_x);
            self.cancel_interpolation();
            return;
        }
        let unchanged = self.is_interpolating()
            && self.target == next_pos
            && self.target_rotation == [next_y, next_x];
        if !unchanged {
            self.target = next_pos;
            self.target_rotation = [next_y, next_x];
            self.steps = self.interp_length;
            self.last_pos = self.position;
            self.last_rot = self.rotation;
        }
    }
    pub fn move_to(&mut self, position: [f64; 3], rotation: Option<(f32, f32)>) {
        self.move_to_opt(Some(position), rotation);
    }
    /// `ClientPacketListener.handleEntityPositionSync`: checks `isTicking` and distance `> 4096`
    /// from current position. Returns whether the display is interpolating afterward.
    pub(in crate::game::client_side) fn apply_position_sync(
        &mut self,
        position: [f64; 3],
        rotation: Option<(f32, f32)>,
        is_ticking: bool,
    ) -> bool {
        let to = glam::DVec3::from(position);
        let too_big = self.position.distance_squared(to) > 4096.0;
        if is_ticking && !too_big {
            self.move_to(position, rotation);
        } else {
            let (y, x) = rotation.unwrap_or((self.rotation[0], self.rotation[1]));
            self.snap_to(to, y, x);
        }
        self.is_interpolating()
    }
    /// `Entity.setValuesFromPositionPacket(change, relatives, interpolate = true)`:
    /// resolves relatives against `getClientPositionAndRotation`, checks `> 4096` from it,
    /// and on a non-interpolated jump resolves `oldPosition`/`yRotO`/`xRotO` independently.
    /// Returns `(resolved_position, interpolated)`.
    pub fn apply_teleport_packet(
        &mut self,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
        relatives: i32,
    ) -> ([f64; 3], bool) {
        use sunrise_protocol::packets::play::relative as rel;
        let resolve = |base_pos: glam::DVec3, base_rot: [f32; 2]| {
            let pick = |bit: i32, cur: f64, chg: f64| {
                if relatives & bit != 0 { cur + chg } else { chg }
            };
            let pos = glam::DVec3::new(
                pick(rel::X, base_pos.x, position[0]),
                pick(rel::Y, base_pos.y, position[1]),
                pick(rel::Z, base_pos.z, position[2]),
            );
            let y = if relatives & rel::Y_ROT != 0 {
                base_rot[0] + yaw
            } else {
                yaw
            };
            let x = clamp_pitch(if relatives & rel::X_ROT != 0 {
                base_rot[1] + pitch
            } else {
                pitch
            });
            (pos, y, x)
        };
        let (cur_pos, cur_rot) = self.client_pos_rot();
        let (new_pos, new_y, new_x) = resolve(cur_pos, cur_rot);
        let too_big = cur_pos.distance_squared(new_pos) > 4096.0;
        if !too_big {
            self.move_to(new_pos.to_array(), Some((new_y, new_x)));
            (new_pos.to_array(), true)
        } else {
            self.set_pos(new_pos);
            self.set_y_rot_raw(new_y);
            self.set_x_rot_raw(new_x);
            let (old_pos, old_y, old_x) = resolve(self.previous, self.old_rotation);
            self.previous = old_pos;
            self.old_rotation = [old_y, old_x];
            (new_pos.to_array(), false)
        }
    }
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    pub(in crate::game::client_side) fn move_remote_text_display(
        &mut self,
        id: i32,
        position: [f64; 3],
        rotation: Option<(f32, f32)>,
    ) {
        if let Some(d) = self.client.text_displays.0.get_mut(&id) {
            d.move_to(position, rotation);
        }
    }
    pub(in crate::game) fn move_remote_text_display_opt(
        &mut self,
        id: i32,
        position: Option<[f64; 3]>,
        rotation: Option<(f32, f32)>,
    ) {
        if let Some(d) = self.client.text_displays.0.get_mut(&id) {
            d.move_to_opt(position, rotation);
        }
    }
    /// `ClientPacketListener.handleEntityPositionSync` for displays.
    pub(in crate::game) fn position_sync_remote_text_display(
        &mut self,
        id: i32,
        position: [f64; 3],
        rotation: Option<(f32, f32)>,
    ) -> bool {
        let Some(pos) = self
            .client
            .text_displays
            .0
            .get(&id)
            .map(RemoteTextDisplay::position)
        else {
            return false;
        };
        let is_ticking = self.is_display_ticking(pos);
        let interpolating = self
            .client
            .text_displays
            .0
            .get_mut(&id)
            .is_some_and(|d| d.apply_position_sync(position, rotation, is_ticking));
        if !interpolating {
            self.file_snapped_display(id);
        }
        if !interpolating && self.display_has_indirect_local_player(id as u32) {
            self.seat_local_player_after_display_snap();
        }
        true
    }
    /// `ClientPacketListener.handleTeleportEntity` for displays.
    pub(in crate::game::client_side) fn teleport_remote_text_display(
        &mut self,
        id: i32,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
        relatives: i32,
    ) -> Option<[f64; 3]> {
        let (resolved, interpolated) = self
            .client
            .text_displays
            .0
            .get_mut(&id)?
            .apply_teleport_packet(position, yaw, pitch, relatives);
        if let Some(r) = self.client.remote.as_mut() {
            r.set_tracked_position(id, resolved);
        }
        if !interpolated {
            self.file_snapped_display(id);
        }
        if !interpolated && self.display_has_indirect_local_player(id as u32) {
            self.seat_local_player_after_display_snap();
        }
        Some(resolved)
    }
    /// A display a packet moved without interpolation changes section at once (`Entity.setPos`
    /// through the level callback), before its riders are seated.
    pub(in crate::game::client_side) fn file_snapped_display(&mut self, id: i32) {
        if let Some(at) = self
            .client
            .text_displays
            .0
            .get(&id)
            .map(RemoteTextDisplay::position)
        {
            self.pick_section_move(id as u32, [at.x, at.y, at.z]);
        }
    }
    /// Re-seats the local player and snaps `previous_position` after a non-interpolated vehicle
    /// snap (`handleEntityPositionSync` / `handleTeleportEntity`).
    pub(in crate::game::client_side) fn seat_local_player_after_display_snap(&mut self) {
        self.position_riders();
        if let Some(id) = self.common.riding {
            let index = std::collections::HashMap::new();
            if let Some(at) = self.passenger_riding_position(id, None, None, &index)
                && let Some(p) = self.common.player.as_mut()
            {
                let scale =
                    p.dimensions.width / sunrise_sim::pose::Pose::Standing.dimensions(1.0).width;
                let (seat, _) = super::remote_vehicles::generic_player_seat(at, at, scale);
                p.position = seat;
                self.common.previous_position = seat;
                // The rider's section follows its seat before the snap returns.
                let rider = p.entity_id as u32;
                self.pick_section_move(rider, seat.components());
            }
        }
    }
    /// `ClientLevel.isTickingEntity`: loaded chunk column membership on a remote client.
    pub(in crate::game::client_side) fn is_display_ticking(&self, pos: glam::DVec3) -> bool {
        if self.client.remote.is_none() {
            return true;
        }
        let cx = pos.x.floor() as i32 >> 4;
        let cz = pos.z.floor() as i32 >> 4;
        self.common.world.chunks.contains_key(&(cx, cz))
    }
    pub(in crate::game::client_side) fn client_entity_vehicle(&self, id: u32) -> Option<u32> {
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.passenger_vehicle(id))
    }
    pub(in crate::game::client_side) fn entity_has_ancestor(
        &self,
        passenger: u32,
        ancestor: u32,
    ) -> bool {
        let mut cur = passenger;
        for _ in 0..16 {
            let Some(v) = self.client_entity_vehicle(cur) else {
                return false;
            };
            if v == ancestor {
                return true;
            }
            cur = v;
        }
        false
    }
    pub(in crate::game::client_side) fn entity_is_player(&self, id: u32) -> bool {
        self.client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id as u32 == id || r.level_players.contains_key(&(id as i32)))
    }
    pub(in crate::game::client_side) fn entity_position_for_tick(
        &self,
        id: u32,
    ) -> Option<glam::DVec3> {
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id as u32 == id)
        {
            return self
                .common
                .player
                .as_ref()
                .map(|p| glam::DVec3::new(p.position.x, p.position.y, p.position.z));
        }
        if let Some(d) = self.client.text_displays.0.get(&(id as i32)) {
            return Some(d.position);
        }
        if let Some(i) = self.common.mob_index.find_ref(&self.common.mobs, id) {
            let m = &self.common.mobs[i];
            return Some(glam::DVec3::new(m.position.x, m.position.y, m.position.z));
        }
        if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
            return Some(glam::DVec3::new(b.position.x, b.position.y, b.position.z));
        }
        if let Some(c) = self.common.carts.iter().find(|c| c.id == id) {
            return Some(glam::DVec3::new(c.position.x, c.position.y, c.position.z));
        }
        if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id) {
            return Some(glam::DVec3::new(a.position.x, a.position.y, a.position.z));
        }
        if let Some(i) = self.client.interactions.get(&(id as i32)) {
            return Some(glam::DVec3::new(i.position.x, i.position.y, i.position.z));
        }
        None
    }
    /// `Entity.hasIndirectPassenger(minecraft.player)`.
    pub(in crate::game::client_side) fn display_has_indirect_local_player(&self, id: u32) -> bool {
        let Some(first) = self.common.riding else {
            return false;
        };
        if first == id {
            return true;
        }
        let mut cur = first;
        for _ in 0..16 {
            let Some(next) = self.client_entity_vehicle(cur) else {
                return false;
            };
            if next == id {
                return true;
            }
            cur = next;
        }
        false
    }
    /// `Entity.countPlayerPassengers() > 0`.
    pub(in crate::game::client_side) fn entity_has_indirect_player(&self, id: u32) -> bool {
        if self.display_has_indirect_local_player(id) {
            return true;
        }
        let Some(r) = self.client.remote.as_ref() else {
            return false;
        };
        for &pid in r.level_players.keys() {
            if pid != r.entity_id && self.entity_has_ancestor(pid as u32, id) {
                return true;
            }
        }
        false
    }
    /// `TickRateManager.isEntityFrozen(display)`.
    pub(in crate::game::client_side) fn is_display_frozen(&self, id: i32, running: bool) -> bool {
        !running && !self.entity_has_indirect_player(id as u32)
    }
    /// `LevelExtractor.extractVisibleEntities` per-entity partial tick.
    pub(in crate::game::client_side) fn display_partial(
        &self,
        id: i32,
        fallback_alpha: f64,
    ) -> f64 {
        if self.common.clock.runs_normally() {
            fallback_alpha
        } else if self.is_display_frozen(id, false) {
            self.common.clock.fraction()
        } else {
            self.common.clock.camera_fraction()
        }
    }
    /// `ClientLevel.tickEntities` + `tickPassenger`: whether this display ticks on this client tick.
    pub(in crate::game::client_side) fn display_should_tick(
        &self,
        id: i32,
        display: &RemoteTextDisplay,
        running: bool,
    ) -> bool {
        if !self.is_display_ticking(display.position) {
            return false;
        }
        let Some(mut cur) = display.vehicle else {
            return !self.is_display_frozen(id, running);
        };
        // Passenger ticks recursively from its root vehicle (`ClientLevel.tickPassenger`):
        // each ancestor must exist and be ticking, and the root vehicle must not be frozen.
        let mut root = cur;
        for _ in 0..16 {
            let Some(pos) = self.entity_position_for_tick(cur) else {
                return false;
            };
            if !self.entity_is_player(cur) && !self.is_display_ticking(pos) {
                return false;
            }
            root = cur;
            match self.client_entity_vehicle(cur) {
                Some(next) if next != cur => cur = next,
                _ => break,
            }
        }
        running || self.entity_is_player(root) || self.entity_has_indirect_player(root)
    }
    /// The displays' tick on a client (`ClientLevel.tickEntities` + `tickPassenger`).
    /// EntityTickEvent: once for the family, or each alone of a type a mod named, right before
    /// and right after its own tick; a canceled one keeps its state.
    pub(in crate::game::client_side) fn tick_and_file_display(&mut self, id: i32) {
        if let Some(display) = self.client.text_displays.0.get_mut(&id) {
            let from = display.position();
            display.tick();
            let at = display.position();
            if at != from {
                self.pick_section_move(id as u32, at.to_array());
            }
        }
    }

    pub fn tick_text_displays(&mut self, running: bool) {
        let mut eligible = Vec::with_capacity(self.client.text_displays.0.len());
        for (&id, display) in &self.client.text_displays.0 {
            if self.display_should_tick(id, display, running) {
                eligible.push(id);
            }
        }
        if eligible.is_empty() {
            return;
        }
        let mut round = self.native_tick_round();
        if !round.told_listening() {
            for id in eligible {
                self.count_client_tick(id as u32);
                self.tick_and_file_display(id);
            }
            return;
        }
        for id in eligible {
            let Some(kind) = self.client.text_displays.0.get(&id).map(|d| d.shows.kind()) else {
                continue;
            };
            // A passenger's ride visit counts it before its own Pre.
            let passenger = self.client_passenger(id as u32);
            if passenger {
                self.count_client_tick(id as u32);
            }
            if self.native_tick_pre_one(&mut round, id as u32, kind) {
                if !passenger {
                    self.count_client_tick(id as u32);
                }
                self.tick_and_file_display(id);
                self.native_tick_post_one(&round, id as u32, kind);
            }
        }
    }
    /// Active render `Camera` yaw and pitch (`DisplayRenderer.extractRenderState`), sharing the
    /// same camera source of truth as `particle_rotation` (`ThirdPersonFront` already baked in).
    pub fn display_camera_angles(&self) -> [f32; 2] {
        if self.client.studio.flying() {
            return [
                self.client.studio.freecam.yaw,
                self.client.studio.freecam.pitch,
            ];
        }
        let (yaw, pitch, _) = self.client.posed_rotation.unwrap_or((
            self.client.view.y_rot,
            self.client.view.x_rot,
            0.0,
        ));
        [yaw, pitch]
    }
    /// `DisplayRenderer.getBlockLightLevel` and `getSkyLightLevel`: the
    /// brightness override, else the light at the entity (block light 15 when on fire).
    pub(in crate::game::client_side) fn display_light(
        world: &sunrise_level::world::World,
        d: &TextDisplayData,
        position: glam::DVec3,
        on_fire: bool,
    ) -> u32 {
        if d.brightness == -1 {
            let mut world_light = world.light(
                position.x.floor() as i32,
                position.y.floor() as i32,
                position.z.floor() as i32,
            );
            if on_fire {
                world_light |= 0x0F;
            }
            sunrise_client::block_entity_render::vertex_light(world_light)
        } else {
            ((d.brightness as u32 >> 20 & 15) * 16) | (((d.brightness as u32 >> 4 & 15) * 16) << 8)
        }
    }
    /// `Display.shouldRenderAtSqrDistance` (strict `<`, uninterpolated entity position, live
    /// `view_range`) and, when culling is active, `EntityRenderer.shouldRender` with
    /// `Display.getInterpolatedBoundingBox` inflated by 0.5 and `±2` fallback only on NaN or
    /// zero size.
    pub(in crate::game::client_side) fn display_in_view_with_frustum(
        camera: glam::DVec3,
        display: &RemoteTextDisplay,
        partial: f32,
        view_scale: f64,
        frustum: Option<&sunrise_client::gfx::JoinedFrusta>,
    ) -> bool {
        if !display.in_range(camera, view_scale) {
            return false;
        }
        if display.no_culling {
            return true;
        }
        let (mut lo, mut hi) = display.interpolated_bounding_box(partial);
        lo -= glam::DVec3::splat(0.5);
        hi += glam::DVec3::splat(0.5);
        let size = ((hi.x - lo.x) + (hi.y - lo.y) + (hi.z - lo.z)) / 3.0;
        if lo.is_nan() || hi.is_nan() || size == 0.0 {
            lo = display.position - glam::DVec3::splat(2.0);
            hi = display.position + glam::DVec3::splat(2.0);
        }
        frustum.is_none_or(|view| view.intersects((lo - camera).as_vec3(), (hi - camera).as_vec3()))
    }
    /// Unified visibility decision (`LevelExtractor.extractVisibleEntities` + `isEntityVisible`),
    /// shared by block/item/text display geometry, display name tags, and display shadows.
    pub(in crate::game::client_side) fn display_visible_with_frustum(
        &self,
        id: i32,
        display: &RemoteTextDisplay,
        camera: glam::DVec3,
        partial: f32,
        scale: f64,
        now: std::time::Instant,
        frustum: Option<&sunrise_client::gfx::JoinedFrusta>,
    ) -> bool {
        if self.hides_as_camera(id as u32) {
            return false;
        }
        if !self.entity_section_visible(display.position, now) {
            return false;
        }
        if self.display_has_indirect_local_player(id as u32) {
            return true;
        }
        Self::display_in_view_with_frustum(camera, display, partial, scale, frustum)
    }
    pub(in crate::game::client_side) fn display_visible(
        &self,
        id: i32,
        display: &RemoteTextDisplay,
        camera: glam::DVec3,
        partial: f32,
        scale: f64,
        now: std::time::Instant,
    ) -> bool {
        self.display_visible_with_frustum(
            id,
            display,
            camera,
            partial,
            scale,
            now,
            self.client.view_frustum.as_ref(),
        )
    }
    /// `Display.getTeamColor` + `EntityRenderer.extractRenderState` outline color: live
    /// `glow_color` override when != -1, else team color, when shared flag 6 is set.
    pub(in crate::game::client_side) fn display_outline(
        &self,
        id: i32,
        display: &RemoteTextDisplay,
    ) -> Option<u32> {
        let vanilla = display.is_glowing().then(|| {
            if display.data.glow_color != -1 {
                display.data.glow_color as u32 & 0xFF_FFFF
            } else {
                super::glowing::team_color(self.client_mob_team(id as u32))
            }
        });
        self.entity_outline(id as u32, display.kind(), vanilla)
    }

    /// `DisplayRenderer.BlockDisplayRenderer` and `ItemDisplayRenderer`: the
    /// block model or the item under the display's transformation, lit like
    /// any entity and never tinted by a hurt overlay.
    pub fn block_item_display_geometry(
        &mut self,
        camera: glam::DVec3,
        partial: f64,
        out: &mut Vec<Vertex>,
    ) {
        let angles = self.display_camera_angles();
        let scale = self.view_scale();
        let now = std::time::Instant::now();
        let mut slots = std::mem::take(&mut self.client.block_display_parts_scratch);
        let mut translucent = std::mem::take(&mut self.client.translucent_display_collector);
        let mut visible = std::mem::take(&mut self.client.display_visible_scratch);
        translucent.clear();
        for shows in [Shows::Block, Shows::Item] {
            let kind = if shows == Shows::Block {
                "minecraft:block_display"
            } else {
                "minecraft:item_display"
            };
            self.mark_entity(out.len(), kind);
            // Evaluate visibility and per-entity partial while `self.text_displays` is intact
            // so indirect-passenger vehicle chains across displays resolve accurately.
            visible.clear();
            for (&id, display) in self
                .client
                .text_displays
                .0
                .iter()
                .filter(|(_, d)| d.shows == shows)
            {
                let entity_partial = self.display_partial(id, partial);
                if self.display_visible(id, display, camera, entity_partial as f32, scale, now) {
                    visible.push((id, entity_partial));
                }
            }
            for &(id, entity_partial) in &visible {
                let partial_f32 = entity_partial as f32;
                let (d, position, rotation, on_fire, shown_block, shown_item, shown_item_display) = {
                    let Some(display) = self.client.text_displays.0.get_mut(&id) else {
                        continue;
                    };
                    // DisplayRenderer.extractRenderState evaluates progress even before renderState != null.
                    let progress = display.calculate_interpolation_progress(partial_f32);
                    if !display.render_state_ready {
                        continue;
                    }
                    (
                        display.eval_render_state(progress),
                        lerp_dvec3(display.previous, display.position, entity_partial),
                        display.rotation_at(partial_f32),
                        display.shared_flags & 0x01 != 0,
                        display.shown_block,
                        display.shown_item.clone(),
                        display.shown_item_display,
                    )
                };
                let pose = d.pose((position - camera).as_vec3().to_array(), rotation, angles);
                if !pose.m.into_iter().all(f32::is_finite) {
                    continue;
                }
                let light = Self::display_light(&self.common.world, &d, position, on_fire);
                let outline = self
                    .client
                    .text_displays
                    .0
                    .get(&id)
                    .and_then(|display| self.display_outline(id, display));
                if let Some(state) = shown_block {
                    let effective_light = self.block_display_light(state, light);
                    translucent.scratch.clear();
                    // BlockModelResolver.update: model seed 42, single resolved sheet, JOML pose.
                    let is_translucent = self.block_display_model_vertices(
                        state,
                        42,
                        pose,
                        effective_light,
                        &mut slots,
                        &mut translucent.scratch,
                    );
                    // CompositeBlockModel.update: special model receives the same resolved light.
                    if self
                        .common
                        .res
                        .facts()
                        .get(state)
                        .name
                        .strip_prefix("minecraft:")
                        .is_some_and(has_special_model)
                    {
                        self.special_block_model(
                            state,
                            pose.to_glam(),
                            super::minecarts_client::special_light(effective_light),
                            &mut translucent.scratch,
                        );
                    }
                    if let Some(rgb) = outline
                        && !translucent.scratch.is_empty()
                    {
                        self.mark_outline(&translucent.scratch, rgb);
                    }
                    if is_translucent {
                        let start = translucent.verts.len();
                        translucent.verts.extend_from_slice(&translucent.scratch);
                        let c = pose.transform_position([0.5, 0.5, 0.5]);
                        let dist_sq = submit_distance_sq(c[0], c[1], c[2]);
                        translucent.push(dist_sq, 0, start..translucent.verts.len());
                    } else {
                        out.extend_from_slice(&translucent.scratch);
                    }
                } else if let Some(stack) = &shown_item {
                    let context = match shown_item_display {
                        1 => DisplayContext::ThirdPersonLeftHand,
                        2 => DisplayContext::ThirdPersonRightHand,
                        3 => DisplayContext::FirstPersonLeftHand,
                        4 => DisplayContext::FirstPersonRightHand,
                        5 => DisplayContext::Head,
                        6 => DisplayContext::Gui,
                        7 => DisplayContext::Ground,
                        8 => DisplayContext::Fixed,
                        9 => DisplayContext::OnShelf,
                        _ => DisplayContext::None,
                    };
                    // updateForNonLiving: seeded by the entity id; rotateY(PI) in JOML order.
                    let state = self.resolve_item(stack, context, None, id);
                    let mut item_pose = pose;
                    item_pose.rotate_y(std::f32::consts::PI);
                    translucent.scratch.clear();
                    self.submit_item_world_joml(&state, item_pose, light, &mut translucent.scratch);
                    if let Some(rgb) = outline
                        && !translucent.scratch.is_empty()
                    {
                        self.mark_outline(&translucent.scratch, rgb);
                    }
                    let tr_start = translucent.verts.len();
                    for quad in translucent.scratch.as_chunks::<4>().0 {
                        let ov = quad[0].layer & 0xffff;
                        if ov == TRANSLUCENT_ITEM_OVERLAY || ov == TRANSLUCENT_GLINT_ITEM_OVERLAY {
                            translucent.verts.extend_from_slice(quad);
                        } else {
                            out.extend_from_slice(quad);
                        }
                    }
                    if translucent.verts.len() > tr_start {
                        let dist_sq =
                            submit_distance_sq(item_pose.m[12], item_pose.m[13], item_pose.m[14]);
                        translucent.push(dist_sq, 1, tr_start..translucent.verts.len());
                    }
                }
            }
        }
        translucent.flush(out);
        self.mark_entity(out.len(), "");
        self.client.display_visible_scratch = visible;
        self.client.block_display_parts_scratch = slots;
        self.client.translucent_display_collector = translucent;
    }
    pub fn text_display_geometry(
        &mut self,
        camera: glam::DVec3,
        partial: f64,
        normal: &mut Vec<Vertex>,
        see_through: &mut SeeThroughCollector,
    ) {
        self.client.text_display_vertices = 0;
        self.mark_translucent_entity(normal.len(), "minecraft:text_display");
        let angles = self.display_camera_angles();
        let scale = self.view_scale();
        let background = self.client.settings.text_background(0.25);
        let now = std::time::Instant::now();
        let mut inline_profiles = std::mem::take(&mut self.client.display_inline_profiles_scratch);
        inline_profiles.clear();
        let mut visible = std::mem::take(&mut self.client.display_visible_scratch);
        visible.clear();
        for (&id, display) in self
            .client
            .text_displays
            .0
            .iter()
            .filter(|(_, d)| d.shows == Shows::Text)
        {
            let entity_partial = self.display_partial(id, partial);
            if self.display_visible(id, display, camera, entity_partial as f32, scale, now) {
                visible.push((id, entity_partial));
            }
        }
        for &(id, entity_partial) in &visible {
            let partial_f32 = entity_partial as f32;
            let (d, position, rotation, on_fire, lines) = {
                let Some(display) = self.client.text_displays.0.get_mut(&id) else {
                    continue;
                };
                // DisplayRenderer.extractRenderState evaluates progress even before renderState != null.
                let progress = display.calculate_interpolation_progress(partial_f32);
                if !display.render_state_ready {
                    continue;
                }
                let d = display.eval_render_state(progress);
                if d.text.is_empty() {
                    continue;
                }
                (
                    d,
                    lerp_dvec3(display.previous, display.position, entity_partial),
                    display.rotation_at(partial_f32),
                    display.shared_flags & 0x01 != 0,
                    display.lines.take(),
                )
            };
            let mut cached = match lines {
                Some(mut cached) => {
                    cached.update(&self.client.gui.atlas, &d, background);
                    cached
                }
                None => CachedQuads::new(&self.client.gui.atlas, &d, background),
            };
            self.client.map_patches.append(&mut cached.batch.patches);
            inline_profiles.append(&mut cached.batch.inline_profiles);
            let mut pose = d.pose((position - camera).as_vec3().to_array(), rotation, angles);
            cached.lines.apply_pose(&mut pose);
            let valid = pose.m.into_iter().all(f32::is_finite) && !cached.batch.vertices.is_empty();
            if valid {
                let batch = &cached.batch;
                let light = Self::display_light(&self.common.world, &d, position, on_fire);
                // Font.DisplayMode: SEE_THROUGH sorts per submit; POLYGON_OFFSET splits opaque text
                // (`canRenderAsSolid`) from backgrounds and translucent text.
                let through = d.flags & 2 != 0;
                let solid_text = d.opacity == -1;
                let st_start = see_through.start();
                let out = if through {
                    see_through.buffer_mut()
                } else {
                    &mut *normal
                };
                for (quad_idx, quad) in batch.vertices.as_chunks::<6>().0.iter().enumerate() {
                    let is_bg = quad_idx * 6 < cached.bg_vertices;
                    let layer = if through {
                        2
                    } else if !is_bg && solid_text {
                        TEXT_POLYGON_OFFSET_SOLID_OVERLAY
                    } else {
                        TEXT_POLYGON_OFFSET_OVERLAY
                    };
                    for i in text_display::CORNERS {
                        let v = quad[i];
                        let pos = pose.transform_position([v.pos[0], v.pos[1], v.depth]);
                        out.push(Vertex {
                            pos,
                            uv: v.uv,
                            color: v.color,
                            light,
                            layer,
                            normal: [0; 4],

                            glint_uv: [0.0, 0.0],
                        });
                    }
                }
                if through {
                    let dist_sq = submit_distance_sq(pose.m[12], pose.m[13], pose.m[14]);
                    see_through.finish(st_start, dist_sq);
                }
                self.client.text_display_vertices += batch.vertices.len() / 6 * 4;
            }
            if let Some(display) = self.client.text_displays.0.get_mut(&id) {
                display.lines = Some(cached);
            }
        }
        self.client.display_visible_scratch = visible;
        if !inline_profiles.is_empty() {
            self.resolve_inline_profiles(&inline_profiles);
        }
        inline_profiles.clear();
        self.client.display_inline_profiles_scratch = inline_profiles;
    }
}

/// Whether vanilla's built-in block models give this block a special model,
/// drawn by its block-entity renderer.
pub(in crate::game::client_side) fn has_special_model(block: &str) -> bool {
    matches!(
        block,
        "bell"
            | "conduit"
            | "decorated_pot"
            | "enchanting_table"
            | "end_gateway"
            | "end_portal"
            | "chest"
            | "trapped_chest"
            | "ender_chest"
            | "shulker_box"
    ) || block.ends_with("_banner")
        || block.ends_with("_shulker_box")
        || block.ends_with("_skull")
        || (block.ends_with("_head") && block != "piston_head")
        || block.contains("copper_chest")
        || block.contains("copper_golem_statue")
}

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    #[test]
    fn culled_display_keeps_quads_across_frame_rollover() {
        let atlas = sunrise_client::hud::HudAtlas::boot();
        let mut display = RemoteTextDisplay::new(Shows::Text, [0.0, 0.0, 5.0], 0.0);
        let d = TextDisplayData {
            text: std::sync::Arc::new(sunrise_game_data::text::Text::literal("Culled")),
            ..Default::default()
        };
        let cache = CachedQuads::new(&atlas, &d, 0);
        let allocation = cache.batch.vertices.as_ptr();
        display.lines = Some(cache);
        for _ in 0..2 {
            let cached = display
                .lines
                .take()
                .filter(|c| c.matches(&d, 0, atlas.font.epoch(), atlas.revision))
                .unwrap_or_else(|| panic!("culled display rebuilt its glyphs"));
            let cached = display.lines.insert(cached);
            let matrix = glam::Mat4::from_translation(glam::Vec3::Z * 5.0) * cached.lines.pose();
            let (lo, _) = cached.transformed_bounds(matrix);
            assert!(lo.z > 0.0, "the display is behind the camera");
            assert_eq!(cached.batch.vertices.as_ptr(), allocation);
        }
    }

    #[test]
    fn cached_text_quads_match_fresh_and_invalidate() {
        let atlas = sunrise_client::hud::HudAtlas::boot();
        let mut d = TextDisplayData {
            text: std::sync::Arc::new(sunrise_game_data::text::Text::literal("Hologram")),
            ..Default::default()
        };
        let cache = CachedQuads::new(&atlas, &d, 0);
        let mut fresh = sunrise_client::hud::HudBatch::scratch([1, 1]);
        text_display::draw(&mut fresh, &atlas, &d, &cache.lines, 0);
        let vertices = |b: &sunrise_client::hud::HudBatch| {
            b.vertices
                .iter()
                .map(|v| (v.pos, v.uv, v.color, v.depth))
                .collect::<Vec<_>>()
        };
        assert_eq!(vertices(&cache.batch), vertices(&fresh));
        let matches = |d: &TextDisplayData| cache.matches(d, 0, atlas.font.epoch(), atlas.revision);
        assert!(matches(&d));
        for change in 0..7 {
            let mut changed = d.clone();
            match change {
                0 => {
                    changed.text =
                        std::sync::Arc::new(sunrise_game_data::text::Text::literal("Changed"))
                }
                1 => changed.text = std::sync::Arc::new((*d.text).clone().bold()),
                2 => changed.opacity = 40,
                3 => changed.background = 0,
                4 => changed.flags = 1,
                5 => changed.flags = 2,
                _ => changed.line_width = 10,
            }
            assert!(!matches(&changed));
        }
        assert!(!cache.matches(&d, 1, atlas.font.epoch(), atlas.revision));
        assert!(!cache.matches(&d, 0, atlas.font.epoch() + 1, atlas.revision));
        assert!(!cache.matches(&d, 0, atlas.font.epoch(), atlas.revision + 1));
        d.text = std::sync::Arc::new(sunrise_game_data::text::Text::legacy("§kSecret"));
        let cache = CachedQuads::new(&atlas, &d, 0);
        assert!(!cache.matches(&d, 0, atlas.font.epoch(), atlas.revision));
        // A plugin hologram sends `§k` inside a literal JSON string: the font
        // resolves it, so its glyphs still change every frame.
        d.text = std::sync::Arc::new(sunrise_game_data::text::Text::literal("§kSecret"));
        let cache = CachedQuads::new(&atlas, &d, 0);
        assert!(!cache.matches(&d, 0, atlas.font.epoch(), atlas.revision));
    }

    #[test]
    fn transformed_text_bounds_reach_into_view_but_not_from_behind() {
        let atlas = sunrise_client::hud::HudAtlas::boot();
        let d = TextDisplayData {
            text: std::sync::Arc::new(sunrise_game_data::text::Text::literal("Wide hologram")),
            ..Default::default()
        };
        let cache = CachedQuads::new(&atlas, &d, 0);
        // A 90-degree camera looking down -Z has side planes x +/- z,
        // y +/- z, and the near plane z = -0.05.
        let visible = |matrix| {
            let (lo, hi) = cache.transformed_bounds(matrix);
            hi.x >= lo.z && lo.x <= -lo.z && hi.y >= lo.z && lo.y <= -lo.z && lo.z <= -0.05
        };
        let pose = |x, z, scale| {
            glam::Mat4::from_translation(glam::Vec3::new(x, 0.0, z))
                * glam::Mat4::from_scale(glam::Vec3::splat(scale))
                * cache.lines.pose()
        };
        assert!(visible(pose(6.0, -5.0, 20.0)));
        assert!(!visible(pose(100.0, -5.0, 1.0)));
        assert!(!visible(pose(0.0, 5.0, 1.0)));
    }

    #[test]
    fn metadata_and_position_interpolation_keep_their_separate_clocks() {
        let mut d = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
        d.metadata(&[(11, 0, Value::Floats(vec![0.0, 0.0, 0.0]))]);
        d.tick();
        d.metadata(&[
            (8, 1, Value::Int(0)),
            (9, 1, Value::Int(4)),
            (10, 1, Value::Int(2)),
            (11, 0, Value::Floats(vec![4.0, 0.0, 0.0])),
        ]);
        d.tick();
        assert_eq!(d.shown.blend(&d.from, d.progress(0.5)).translation[0], 0.5);
        d.move_to([2.0, 0.0, 0.0], Some((90.0, 0.0)));
        d.tick();
        assert_eq!(d.position.x, 1.0);
        assert_eq!(d.rotation[0], 45.0);
        d.tick();
        assert_eq!(d.position.x, 2.0);
        assert_eq!(d.shown.blend(&d.from, d.progress(0.0)).translation[0], 2.0);
    }

    /// A display riding a vehicle sits where `positionRider` (`ride`) places it after `tick()`,
    /// moving from the last tick's seat while preserving active rotation interpolation.
    #[test]
    fn a_ridden_display_moves_from_seat_to_seat() {
        let mut d = RemoteTextDisplay::new(Shows::Text, [0.0, 64.0, 0.0], 0.0);
        d.metadata(&[(10, 1, Value::Int(3))]);
        d.move_to([9.0, 64.0, 9.0], Some((90.0, 0.0)));
        let seat = glam::DVec3::new(0.0, 65.8, 0.0);
        d.ride(seat);
        assert_eq!(d.position, seat);
        assert!(d.is_interpolating());
        // Entity.rideTick runs tick() (advancing rotation) then vehicle.positionRider (setPos).
        d.tick();
        d.ride(seat);
        assert_eq!((d.previous, d.position), (seat, seat));
        assert_eq!(d.rotation[0], 30.0);
        let next = glam::DVec3::new(1.0, 65.8, 0.0);
        d.tick();
        d.ride(next);
        assert_eq!((d.previous, d.position), (seat, next));
        assert_eq!(d.rotation[0], 60.0);
    }

    #[test]
    fn movement_packets_distinguish_position_sync_and_relative_teleport() {
        use sunrise_protocol::packets::play::relative as rel;

        // PositionSync from x=0 to x=65 (> 64 blocks) snaps even with a 10-tick duration.
        let mut d = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
        d.metadata(&[(10, 1, Value::Int(10))]);
        assert!(!d.apply_position_sync([65.0, 0.0, 0.0], Some((20.0, 10.0)), true));
        assert_eq!(d.position.x, 65.0);
        assert_eq!(d.previous.x, 65.0);
        assert!(!d.is_interpolating());

        // PositionSync when not ticking snaps even within 64 blocks.
        assert!(!d.apply_position_sync([66.0, 0.0, 0.0], Some((25.0, 0.0)), false));
        assert_eq!(d.position.x, 66.0);

        // Teleport with relative +15° yaw against an active 30° target queues 45°.
        let mut dt = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
        dt.metadata(&[(10, 1, Value::Int(10))]);
        dt.move_to([10.0, 0.0, 0.0], Some((30.0, 0.0)));
        let flags = rel::X | rel::Y | rel::Z | rel::Y_ROT;
        assert!(
            dt.apply_teleport_packet([0.0, 0.0, 0.0], 15.0, 0.0, flags)
                .1
        );
        for _ in 0..10 {
            dt.tick();
        }
        assert_eq!(dt.rotation[0], 45.0);

        // Teleport > 4096 dist² resolves old pose independently on non-interpolated path.
        let mut df = RemoteTextDisplay::new(Shows::Text, [1.0, 2.0, 3.0], 10.0);
        df.snap_to(glam::DVec3::new(1.0, 2.0, 3.0), 10.0, 5.0);
        df.previous = glam::DVec3::new(-4.0, -5.0, -6.0);
        df.old_rotation = [20.0, -10.0];
        assert!(
            !df.apply_teleport_packet([100.0, 0.0, 0.0], 15.0, 0.0, flags)
                .1
        );
        assert_eq!(df.position, glam::DVec3::new(101.0, 2.0, 3.0));
        assert_eq!(df.previous, glam::DVec3::new(96.0, -5.0, -6.0));
        assert_eq!(df.rotation, [25.0, 0.0]);
        assert_eq!(df.old_rotation, [35.0, 0.0]);
    }

    #[test]
    fn rotation_arithmetic_snap_nonfinite_and_partial_endpoints() {
        let mut d = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
        d.metadata(&[(10, 1, Value::Int(3))]);
        d.move_to([0.0; 3], Some((0.0, 7.0)));
        d.tick();
        // Double-precision linear interpolation step: (0.0 + (7.0 - 0.0) / 3.0) as f32.
        assert_eq!(
            d.rotation[1].to_bits(),
            ((7.0_f64 / 3.0_f64) as f32).to_bits()
        );

        d.snap_to(glam::DVec3::ZERO, 400.0, 400.0);
        assert_eq!(d.rotation[0], 400.0);
        assert_eq!(d.rotation[1], 40.0);
        d.set_y_rot_raw(f32::NAN);
        d.set_x_rot_raw(f32::INFINITY);
        assert_eq!(d.rotation, [400.0, 40.0]);

        d.snap_to(glam::DVec3::ZERO, 0.0, 0.0);
        d.old_rotation = [350.0, 0.0];
        assert_eq!(d.rotation_at(1.0)[0].to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn partial_moves_negative_duration_repeat_age_wrap_and_nan_range() {
        // Duration=0 rotation-only packet completes active X target.
        let mut d = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
        d.metadata(&[(10, 1, Value::Int(4))]);
        d.move_to([10.0, 0.0, 0.0], Some((0.0, 0.0)));
        d.tick();
        d.metadata(&[(10, 1, Value::Int(0))]);
        d.move_to_opt(None, Some((45.0, 0.0)));
        assert_eq!(d.position.x, 10.0);
        assert_eq!(d.rotation[0], 45.0);

        // Repeating identical target after setting negative duration preserves active ease.
        let mut dn = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
        dn.metadata(&[(10, 1, Value::Int(4))]);
        dn.move_to([4.0, 0.0, 0.0], Some((0.0, 0.0)));
        dn.tick();
        dn.metadata(&[(10, 1, Value::Int(-2))]);
        dn.move_to([4.0, 0.0, 0.0], Some((0.0, 0.0)));
        dn.tick();
        assert_eq!(dn.position.x, 2.0);

        // Age + start_delta wraps in i32 before widening to i64 (age becomes 2 on tick 2).
        let mut dw = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
        dw.metadata(&[(
            23,
            0,
            Value::Text(sunrise_core::nbt::Tag::String("W".into())),
        )]);
        dw.tick(); // age = 1
        dw.metadata(&[
            (8, 1, Value::Int(i32::MAX)),
            (9, 1, Value::Int(4)),
            (11, 0, Value::Floats(vec![4.0, 0.0, 0.0])),
        ]);
        dw.tick(); // age = 2 -> 2i32.wrapping_add(i32::MAX) == i32::MIN + 1
        assert_eq!(dw.start, i64::from(i32::MIN + 1));
        assert_eq!(dw.progress(0.0), 1.0);

        // NaN view_range is rejected by strict range check.
        dw.metadata(&[(17, 2, Value::Float(f32::NAN))]);
        assert!(!dw.in_range(glam::DVec3::ZERO, 1.0));
    }

    #[test]
    fn see_through_collector_sorts_tags_and_displays_together() {
        let mut collector = SeeThroughCollector::default();
        let make = |c: u32| {
            [sunrise_client::mesher::Vertex {
                pos: [0.0; 3],
                uv: [0.0; 2],
                layer: 0,
                color: c,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.0; 2],
            }; 4]
        };
        let submit = |collector: &mut SeeThroughCollector, dist_sq: f32, verts: &[Vertex]| {
            let start = collector.start();
            collector.buffer_mut().extend_from_slice(verts);
            collector.finish(start, dist_sq);
        };
        submit(&mut collector, submit_distance_sq(0.0, 0.0, -2.0), &make(1)); // near tag
        submit(&mut collector, submit_distance_sq(0.0, 0.0, -5.0), &make(2)); // far display
        submit(&mut collector, submit_distance_sq(0.0, 0.0, -5.0), &make(3)); // tie display
        let mut out = Vec::new();
        collector.flush(&mut out);
        let order: Vec<u32> = out.as_chunks::<4>().0.iter().map(|q| q[0].color).collect();
        assert_eq!(order, vec![2, 3, 1]);
    }
}
