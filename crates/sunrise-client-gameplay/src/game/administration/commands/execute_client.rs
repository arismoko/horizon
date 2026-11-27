//! Client-side entity eye-height lookup (`Entity.getEyeHeight`).
use sunrise_gameplay_common::game::commands_common::Target;

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn target_eye_height(&self, t: Target) -> f64 {
        match t {
            Target::Player => self
                .common
                .player
                .as_ref()
                .map_or(1.62, |p| f64::from(p.dimensions.eye_height)),
            Target::Guest(_) => {
                debug_assert!(false, "client target_eye_height called with Target::Guest");
                1.62
            }
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or(0.0, |m| m.eye_y() - m.position.y),
            // ArmorStand.getDefaultDimensions: a marker's are empty, a small
            // stand's are BABY_DIMENSIONS, whose eye height is 0.9875F.
            Target::Stand(id) => match self.common.armor_stands.iter().find(|a| a.id == id) {
                Some(a) if a.marker => 0.0,
                Some(a) if a.small => f64::from(0.9875f32),
                _ => self.common.type_eye_height("minecraft:armor_stand"),
            },
            Target::Other(id) => self
                .client
                .interactions
                .get(&(id as i32))
                .map_or(0.0, |i| f64::from(i.eye_height())),
            Target::General(id) => {
                let ty = self
                    .client
                    .remote
                    .as_ref()
                    .and_then(|r| r.general.rows.get(&(id as i32)))
                    .map_or("", |row| row.kind());
                self.common.type_eye_height(ty)
            }
            Target::Projectile(id) => {
                let ty = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .map_or("", |p| p.entity_type());
                self.common.type_eye_height(ty)
            }
            Target::Frame(id) => {
                let ty = if self.common.item_frames.iter().any(|f| f.id == id && f.glow) {
                    "minecraft:glow_item_frame"
                } else {
                    "minecraft:item_frame"
                };
                self.common.type_eye_height(ty)
            }
            Target::Boat(id) => {
                let ty = self
                    .common
                    .boats
                    .iter()
                    .find(|b| b.id == id)
                    .map_or_else(|| "minecraft:oak_boat".into(), |b| b.entity_type());
                self.common.type_eye_height(&ty)
            }
            Target::Cart(id) => {
                let ty = self
                    .common
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .map_or("minecraft:minecart", |c| c.kind.id());
                self.common.type_eye_height(ty)
            }
            Target::Item(_) => self.common.type_eye_height("minecraft:item"),
            Target::Orb(_) => self.common.type_eye_height("minecraft:experience_orb"),
            Target::Painting(_) => self.common.type_eye_height("minecraft:painting"),
            Target::Crystal(_) => self.common.type_eye_height("minecraft:end_crystal"),
            Target::Dragon(_) | Target::DragonPart(_) => {
                self.common.type_eye_height("minecraft:ender_dragon")
            }
            Target::Tnt(_) => self.common.type_eye_height("minecraft:tnt"),
            Target::Cushion(_) => self.common.type_eye_height("minecraft:cushion"),
            Target::Falling(_) => self.common.type_eye_height("minecraft:falling_block"),
            Target::Rocket(_) => self.common.type_eye_height("minecraft:firework_rocket"),
            Target::Trident(_) => self.common.type_eye_height("minecraft:trident"),
            Target::Fangs(_) => self.common.type_eye_height("minecraft:evoker_fangs"),
            Target::Spawner(_) => self
                .common
                .type_eye_height("minecraft:ominous_item_spawner"),
            Target::Cloud(_) => self.common.type_eye_height("minecraft:area_effect_cloud"),
            Target::Bolt(_) => self.common.type_eye_height("minecraft:lightning_bolt"),
            Target::Hook(_) => self.common.type_eye_height("minecraft:fishing_bobber"),
            Target::Knot(_) => self.common.type_eye_height("minecraft:leash_knot"),
        }
    }
}
