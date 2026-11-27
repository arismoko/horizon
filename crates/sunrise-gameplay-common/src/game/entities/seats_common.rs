use sunrise_core::geometry::Vec3;

/// `Avatar.DEFAULT_VEHICLE_ATTACHMENT`: a player sits this far below the vehicle's passenger point.
const PLAYER_VEHICLE_ATTACHMENT: f64 = 0.6;

/// Such a vehicle: where it is, and how tall and wide its box is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seat {
    pub position: Vec3,
    pub height: f64,
    pub width: f32,
    pub attachment: [f64; 3],
    pub yaw: f32,
}

impl Seat {
    pub fn general(type_slot: usize, position: Vec3, yaw: f32) -> Self {
        let catalog = super::general::general_type_catalog();
        let (width, height) = catalog.dimensions(type_slot);
        Self {
            position,
            height,
            width: width as f32,
            attachment: catalog.passenger_attachment(type_slot),
            yaw,
        }
    }

    /// `Entity.positionRider`: the passenger point less the rider's vehicle attachment.
    pub fn passenger_position(&self, rider_offset: f64) -> Vec3 {
        sunrise_sim::mobs::riders::seat(self.attachment, rider_offset, self.position, self.yaw)
    }

    /// `Entity.positionRider` for a player: its vehicle attachment below the passenger point.
    pub fn player_seat(&self) -> Vec3 {
        self.passenger_position(PLAYER_VEHICLE_ATTACHMENT)
    }

    pub fn position_mob(&self, mob: &mut sunrise_sim::mobs::Mob) {
        let offset = sunrise_sim::mobs::riders::vehicle_attachment(mob, self.width);
        sunrise_sim::mobs::living::teleports::snap_to(mob, self.passenger_position(offset));
        mob.velocity = Vec3::ZERO;
        mob.fall_distance = 0.0;
        mob.ground.on_ground = false;
        mob.extra.passenger = true;
        mob.ai.nav.stop();
    }
    /// `Entity.getDismountLocationForPassenger`: `(getX(), getBoundingBox().maxY, getZ())`.
    pub fn dismount(&self) -> Vec3 {
        Vec3::new(
            self.position.x,
            self.position.y + self.height,
            self.position.z,
        )
    }
}

/// `Entity.couldAcceptPassenger`: false for exactly `Marker` (`Marker.java:47`) and
/// `OminousItemSpawner` (`OminousItemSpawner.java:134`), and not even a forced `startRiding` or `/ride`
/// bypasses it. A marker armor stand is an ordinary armor stand and accepts.
pub fn could_accept_passenger(kind: &str) -> bool {
    !matches!(
        kind,
        "minecraft:marker" | "marker" | "minecraft:ominous_item_spawner"
    )
}
