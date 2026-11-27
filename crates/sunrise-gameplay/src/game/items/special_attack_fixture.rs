//! Scoped Player.attack follow-up probe; invoked by the dispenser's headless world.
use super::minecarts::{CartKind, Minecart};
use serde_json::Value;
use sunrise_core::geometry::Vec3;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::{boats_common::Boat, dragon_common::Dragon};
use sunrise_items::item_data::parse_stack;
use sunrise_sim::{
    entities::{ArmorStand, ItemFrame, Painting},
    motion::PlayerMotion,
};

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn fixture_special_attack(
        &mut self,
        op: &Value,
    ) -> Result<Vec<String>> {
        let kind = op["target"]
            .as_str()
            .ok_or("special attack target missing")?;
        let creative = op["creative"].as_bool().unwrap_or(false);
        let mut player = PlayerMotion::new(self.common.motion, Vec3::new(0.5, 0.0, -2.0))?;
        player.invulnerable = creative;
        player.may_fly = creative;
        player.ground.on_ground = true;
        player.velocity = Vec3::new(0.1, 0.0, 0.1);
        player.fall_distance = op["fall"].as_f64().unwrap_or(0.0);
        if let Some(damage) = op["attack_damage"].as_f64() {
            let key = player
                .attributes
                .key("minecraft:attack_damage")
                .ok_or("attack damage missing")?;
            let handle = player
                .attributes
                .get_instance(Some(key))
                .map_err(|error| format!("attack damage attribute: {error:?}"))?
                .ok_or("attack damage missing")?;
            player
                .attributes
                .edit(&handle, |value, observer| {
                    value.set_base_value(damage, observer)
                })
                .map_err(|error| format!("attack damage update: {error:?}"))?;
        }
        player.set_sprinting(op["sprint"].as_bool().unwrap_or(false))?;
        self.common.player = Some(player);
        self.common.mode = if creative {
            sunrise_command::args::GameMode::Creative
        } else {
            sunrise_command::args::GameMode::Survival
        };
        self.common.creative = creative;
        self.common.inventory.fill(None);
        self.common.inventory[self.common.selected] =
            parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.common.offhand = None;
        // Every case installed a fresh player attribute map; the previous
        // actor's applied-effect cache must not suppress this actor's modifiers.
        self.common.potions = Default::default();
        if op["weakness"].as_bool().unwrap_or(false) {
            self.add_player_effect(sunrise_level::effects::EffectInstance::new(
                "weakness", 200, 0,
            ));
        }
        self.server.attack_ticks = op["ticks"].as_i64().unwrap_or(20) as i32;
        self.common.world.game_time = 100;
        let at = Vec3::new(0.5, 1.0, 0.5);
        // The fixture targets and the live drops from breaking them share one allocator.
        let center = match kind {
            "boat" => {
                let id = self.take_entity_id();
                self.common.boats.push(Boat::new(id, "oak".into(), at, 0.0));
                self.common.boats[0].bounds().center()
            }
            "cart" => {
                let id = self.take_entity_id();
                self.common
                    .carts
                    .push(Minecart::new(id, CartKind::Rideable, at));
                self.common.carts[0].bounds().center()
            }
            "stand" => {
                let id = self.take_entity_id();
                let mut stand = ArmorStand::new(id, Vec3::new(0.5, 0.0, 0.5), 0.0);
                stand.last_hit = if op["break"].as_bool().unwrap_or(false) {
                    100
                } else {
                    0
                };
                self.common.armor_stands.push(stand);
                self.common.armor_stands[0].bounds().center()
            }
            "dragon" => {
                let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
                for part in &mut dragon.parts {
                    part.0 = Vec3::new(100.0, 100.0, 100.0);
                    part.1 = Vec3::ZERO;
                }
                dragon.parts[0].0 = at;
                dragon.parts[0].1 = at;
                self.common.dragon.replace(dragon);
                self.common.special_targets()[0].1.center()
            }
            "painting" => {
                let id = self.take_entity_id();
                self.common.paintings.push(Painting {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id,
                    pos: [0, 1, 0],
                    facing: 2,
                    variant: "minecraft:kebab".into(),
                    asset: None,
                    width: 1,
                    height: 1,
                    check_interval: 0,
                    ticks: 0,
                });
                let (lo, hi) = self.common.paintings[0].bounds();
                Vec3::new(
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                )
            }
            "frame" | "fixed_frame" | "filled_frame" => {
                let id = self.take_entity_id();
                self.common.item_frames.push(ItemFrame {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id,
                    pos: [0, 1, 0],
                    facing: 2,
                    glow: false,
                    item: if kind == "filled_frame" {
                        Some(sunrise_items::items::ItemStack::new("minecraft:stone", 1))
                    } else {
                        None
                    },
                    rotation: 0,
                    drop_chance: 1.0,
                    fixed: kind == "fixed_frame",
                    invisible: false,
                    check_interval: 0,
                    ticks: 0,
                    custom_name: None,
                    random: sunrise_core::random::LegacyRandom::new(0),
                });
                let (lo, hi) = self.common.item_frames[0].bounds();
                Vec3::new(
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                )
            }
            _ => return Err(format!("unknown special attack target {kind}").into()),
        };
        let eye = self
            .common
            .eye_position()
            .ok_or("special attack player missing")?;
        let dx = center.x - eye[0];
        let dy = center.y - eye[1];
        let dz = center.z - eye[2];
        self.common.yaw = (-dx).atan2(dz).to_degrees() as f32;
        self.common.pitch = (-dy).atan2((dx * dx + dz * dz).sqrt()).to_degrees() as f32;
        self.attack();
        // A ray miss must not pass as an accepted hit with an unchanged cost.
        let accepted = match kind {
            "boat" => Some(self.common.boats.first().is_none_or(|b| b.hurt_time > 0)),
            "cart" => Some(self.common.carts.first().is_none_or(|c| c.hurt_time > 0)),
            "stand" => Some(
                self.common
                    .armor_stands
                    .first()
                    .is_none_or(|s| !op["break"].as_bool().unwrap_or(false) && s.last_hit == 100),
            ),
            "dragon" => Some(
                self.common
                    .dragon
                    .as_ref()
                    .is_some_and(|d| d.health < 200.0),
            ),
            _ => None,
        };
        let outcome = accepted.map_or("control", |hit| if hit { "accepted" } else { "unchanged" });
        let player = self
            .common
            .player
            .take()
            .ok_or("special attack player missing")?;
        Ok(vec![format!(
            "special {outcome} {:08x} {:016x} {:016x} {} {}",
            player.health.food.exhaustion.to_bits(),
            player.velocity.x.to_bits(),
            player.velocity.z.to_bits(),
            player.sprinting,
            self.server.attack_ticks
        )])
    }
}
