use sunrise_core::geometry::{Aabb, Vec3};

/// `Interaction`'s synced data and box (`DATA_WIDTH`, `DATA_HEIGHT`, `DATA_RESPONSE`).
pub struct RemoteInteraction {
    pub join: sunrise_sim::join_seq::JoinSeq,
    pub position: Vec3,
    pub previous: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub previous_yaw: f32,
    pub previous_pitch: f32,
    pub width: f32,
    pub height: f32,
    pub dimensions_refreshed: bool,
    pub response: bool,
    /// `Entity.getVehicle`: ModelEngine's hitboxes ride the pivot that
    /// carries them.
    pub vehicle: Option<u32>,
}

impl RemoteInteraction {
    pub fn new(position: [f64; 3]) -> Self {
        Self::new_with_rotation(position, 0.0, 0.0)
    }
    pub fn new_with_rotation(position: [f64; 3], yaw: f32, pitch: f32) -> Self {
        let pos = Vec3::new(position[0], position[1], position[2]);
        let yaw = if yaw.is_finite() { yaw } else { 0.0 };
        let pitch = if pitch.is_finite() {
            (pitch % 360.0).clamp(-90.0, 90.0)
        } else {
            0.0
        };
        Self {
            join: sunrise_sim::join_seq::JoinSeq::next(),
            position: pos,
            previous: pos,
            yaw,
            pitch,
            previous_yaw: yaw,
            previous_pitch: pitch,
            width: 1.0,
            height: 1.0,
            dimensions_refreshed: false,
            response: false,
            vehicle: None,
        }
    }
    /// Unsynced spawns keep EntityType's 0x0 eye height until DATA_POSE/WIDTH/HEIGHT triggers refreshDimensions.
    pub fn eye_height(&self) -> f32 {
        if self.dimensions_refreshed {
            self.height * 0.85
        } else {
            0.0
        }
    }
    /// Unsynced spawns keep EntityType's 0x0 dimensions until refreshDimensions runs.
    #[cfg(any(test, feature = "test-support"))]
    pub fn seat_height(&self) -> f64 {
        if self.dimensions_refreshed {
            f64::from(self.height)
        } else {
            0.0
        }
    }
    /// `Interaction.refreshDimensions`: `width` square about the entity's
    /// x and z, from its feet up `height` (its position is the box's bottom
    /// centre, not the model's).
    pub fn bounds(&self) -> Aabb {
        let half = f64::from(self.width) / 2.0;
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(p.x + half, p.y + f64::from(self.height), p.z + half),
        )
    }
}

/// Whether an entity id names `minecraft:interaction`.
pub fn is_interaction(kind: &str) -> bool {
    matches!(kind, "minecraft:interaction" | "interaction")
}

/// `Interaction.PlayerAction`: the player and the game time of the last
/// attack or use, as `PlayerAction.CODEC` saves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerAction {
    pub player: [i32; 4],
    pub timestamp: i64,
}

impl PlayerAction {
    fn from_tag(tag: Option<&sunrise_core::nbt::Tag>) -> Option<Self> {
        use sunrise_core::nbt::Tag;
        let tag = tag?;
        let Tag::IntArray(player) = tag.get("player")? else {
            return None;
        };
        let timestamp = sunrise_sim::save::number(tag.get("timestamp"))?;
        Some(Self {
            player: player.as_slice().try_into().ok()?,
            timestamp,
        })
    }
    fn to_tag(self) -> sunrise_core::nbt::Tag {
        use sunrise_core::nbt::Tag;
        Tag::compound([
            ("player", Tag::IntArray(self.player.to_vec())),
            ("timestamp", Tag::Long(self.timestamp)),
        ])
    }
}

/// A server `Interaction`'s saved settings
/// (`Interaction.readAdditionalSaveData` / `addAdditionalSaveData`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionState {
    pub width: f32,
    pub height: f32,
    pub response: bool,
    pub attack: Option<PlayerAction>,
    pub interaction: Option<PlayerAction>,
}

impl InteractionState {
    /// `Interaction.readAdditionalSaveData`: the dimensions, optional action records and response.
    pub fn from_tag(tag: &sunrise_core::nbt::Tag) -> Self {
        let float = |key| sunrise_sim::save::float(tag.get(key)).unwrap_or(1.0) as f32;
        Self {
            width: float("width"),
            height: float("height"),
            response: sunrise_sim::entity_nbt::flag(tag, "response"),
            attack: PlayerAction::from_tag(tag.get("attack")),
            interaction: PlayerAction::from_tag(tag.get("interaction")),
        }
    }
    /// `Interaction.addAdditionalSaveData`: the settings, and each action when set.
    pub fn write(&self, c: &mut sunrise_sim::entity_nbt::EntityTagWriter) {
        use sunrise_core::nbt::Tag;
        c.insert("width".into(), Tag::Float(self.width.to_bits()));
        c.insert("height".into(), Tag::Float(self.height.to_bits()));
        for (key, action) in [("attack", self.attack), ("interaction", self.interaction)] {
            match action {
                Some(action) => {
                    c.insert(key.into(), action.to_tag());
                }
                None => {
                    c.remove(&key.into());
                }
            }
        }
        c.insert("response".into(), Tag::Byte(i8::from(self.response)));
    }
}
