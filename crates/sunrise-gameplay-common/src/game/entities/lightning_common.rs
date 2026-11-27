use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use sunrise_level::random_source;

#[derive(Default)]
pub struct StrikeState {
    /// Mooshrooms and copper golems change once per bolt.
    pub converted: Vec<u32>,
    /// The mobs it has struck (`hitEntities`).
    pub hit: Vec<u32>,
    /// The dropped items in the strike box (`hitEntities` holds every entity).
    pub hit_items: Vec<u32>,
    /// Items a cushion dropped under it, spared for 20 ticks
    /// (`Cushion.dropItem`: `setInvulnerableTime(20)`).
    pub spared: Vec<u32>,
}

pub struct Bolt {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub(in crate::game) join: sunrise_sim::join_seq::JoinSeq,
    /// The entity id a server's players know it by (0 until it has one).
    pub id: u32,
    pub at: [f64; 3],
    pub velocity: Vec3,
    pub rotation: [f32; 2],
    pub fall_distance: f64,
    pub fire_ticks: i32,
    pub air: i32,
    pub on_ground: bool,
    pub invulnerable: bool,
    pub invulnerable_time: i32,
    pub portal_cooldown: i32,
    pub custom_name: Option<sunrise_core::nbt::Tag>,
    pub custom_name_visible: bool,
    pub silent: bool,
    pub no_gravity: bool,
    pub glowing: bool,
    pub ticks_frozen: i32,
    pub has_visual_fire: bool,
    /// Vehicle entity id when this bolt rides a serializable entity (`Entity.vehicle`).
    pub mount: Option<u32>,
    pub vehicle_order: i64,
    pub life: i32,
    pub flashes: i32,
    pub seed: i64,
    /// The bolt's own random (`Entity.random`): UUID, then seed, flashes, flash
    /// timing and fire spots draw from it, never the level's.
    pub random: LegacyRandom,
    pub visual_only: bool,
    pub strike: StrikeState,
    /// Blocks it set on fire (`blocksSetOnFire`).
    pub fires: i32,
    /// The player who channelled it (`LightningBolt.cause`): `Some(None)` for
    /// the host, `Some(Some(conn))` for a guest.
    pub cause: Option<Option<u32>>,
}

sunrise_sim::impl_joined!(Bolt);
sunrise_mod_runtime::impl_attachment_lifetime!(Bolt);

impl Bolt {
    /// `LightningBolt.<init>`: `Entity.<init>` draws the UUID first, then `seed` and `flashes`.
    pub fn new(id: u32, at: [f64; 3], visual_only: bool) -> (Self, [i32; 4]) {
        let mut random = random_source::entity();
        let uuid = sunrise_sim::save::insecure_uuid(&mut random);
        let seed = random.next_long();
        let flashes = random.next_int_bound(3).unwrap_or(0) + 1;
        (
            Self {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                at,
                velocity: Vec3::ZERO,
                rotation: [0.0; 2],
                fall_distance: 0.0,
                fire_ticks: 0,
                air: 300,
                on_ground: false,
                invulnerable: false,
                invulnerable_time: 0,
                portal_cooldown: 0,
                custom_name: None,
                custom_name_visible: false,
                silent: false,
                no_gravity: false,
                glowing: false,
                ticks_frozen: 0,
                has_visual_fire: false,
                mount: None,
                vehicle_order: 0,
                life: 2,
                flashes,
                seed,
                random,
                visual_only,
                strike: StrikeState::default(),
                fires: 0,
                cause: None,
            },
            uuid,
        )
    }

    /// `Entity.load` inherited fields (`LightningBolt.readAdditionalSaveData` is empty).
    pub fn apply_nbt(&mut self, tag: &sunrise_core::nbt::Tag) {
        use sunrise_game_data::nbt_path::int_value;
        use sunrise_sim::{entity_nbt, save::float};
        if let Some(pos) = entity_nbt::position(tag) {
            self.at = [
                pos.x.clamp(-3.000_051_2e7, 3.000_051_2e7),
                pos.y.clamp(-2.0e7, 2.0e7),
                pos.z.clamp(-3.000_051_2e7, 3.000_051_2e7),
            ];
        }
        self.velocity = entity_nbt::loaded_motion(tag);
        self.rotation = entity_nbt::loaded_rotation(tag);
        self.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
        self.fire_ticks = i32::from(
            tag.get("Fire")
                .and_then(int_value)
                .map_or(0_i16, |v| v as i16),
        );
        self.air = tag.get("Air").and_then(int_value).unwrap_or(300);
        self.on_ground = entity_nbt::flag(tag, "OnGround");
        self.invulnerable = entity_nbt::flag(tag, "Invulnerable");
        self.invulnerable_time = tag
            .get("invulnerable_time")
            .and_then(int_value)
            .unwrap_or(0);
        self.portal_cooldown = tag.get("PortalCooldown").and_then(int_value).unwrap_or(0);
        self.custom_name = tag
            .get("CustomName")
            .and_then(entity_nbt::validated_component);
        self.custom_name_visible = entity_nbt::flag(tag, "CustomNameVisible");
        self.silent = entity_nbt::flag(tag, "Silent");
        self.no_gravity = entity_nbt::flag(tag, "NoGravity");
        self.glowing = entity_nbt::flag(tag, "Glowing");
        self.ticks_frozen = tag.get("TicksFrozen").and_then(int_value).unwrap_or(0);
        self.has_visual_fire = entity_nbt::flag(tag, "HasVisualFire");
    }
}
#[cfg(any(test, feature = "test-support"))]
impl Bolt {
    /// A spent bolt at `at`, for tests that call `thunder_hit` directly.
    pub fn struck_at(id: u32, at: [f64; 3]) -> Self {
        Self {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            at,
            velocity: Vec3::ZERO,
            rotation: [0.0; 2],
            fall_distance: 0.0,
            fire_ticks: 0,
            air: 300,
            on_ground: false,
            invulnerable: false,
            invulnerable_time: 0,
            portal_cooldown: 0,
            custom_name: None,
            custom_name_visible: false,
            silent: false,
            no_gravity: false,
            glowing: false,
            ticks_frozen: 0,
            has_visual_fire: false,
            mount: None,
            vehicle_order: 0,
            life: 0,
            flashes: 0,
            seed: 0,
            random: LegacyRandom::new(0),
            visual_only: false,
            strike: StrikeState::default(),
            fires: 0,
            cause: None,
        }
    }
}
