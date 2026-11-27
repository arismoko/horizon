use crate::game::ServerGame;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::end_state::EyeOfEnder;
use sunrise_items::items::ItemStack;

impl ServerGame<'_> {
    /// EnderEyeItem.use: findNearestMapStructure over the stronghold rings,
    /// then an EyeOfEnder signalled towards it. Without a stronghold (other
    /// dimensions) nothing is thrown.
    pub(in crate::game::server_side) fn throw_ender_eye(&mut self) {
        let rings = self
            .common
            .end
            .locator
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|t| t.ring_positions("minecraft:strongholds"));
        let (Some(rings), Some(p)) = (rings, self.common.player.as_ref()) else {
            return;
        };
        let block = [
            p.position.x.floor(),
            p.position.y.floor(),
            p.position.z.floor(),
        ];
        // ChunkGenerator.getNearestGeneratedStructure: the ring chunk closest to
        // its (8, 32, 8) block, located at the chunk's corner.
        let Some(nearest) = rings.iter().min_by(|a, b| {
            let d = |c: &sunrise_core::position::ChunkPos| {
                (f64::from(c.x * 16 + 8) - block[0]).powi(2)
                    + (32.0 - block[1]).powi(2)
                    + (f64::from(c.z * 16 + 8) - block[2]).powi(2)
            };
            d(a).total_cmp(&d(b))
        }) else {
            return;
        };
        let target = glam::DVec3::new(f64::from(nearest.x * 16), 0.0, f64::from(nearest.z * 16));
        let start = glam::DVec3::new(
            p.position.x,
            p.position.y + f64::from(p.dimensions.height) * 0.5,
            p.position.z,
        );
        // EyeOfEnder.signalTo.
        let delta = target - start;
        let horizontal = sunrise_core::geometry::length(delta.x, delta.z);
        let target = if horizontal > 12.0 {
            start
                + glam::DVec3::new(
                    delta.x / horizontal * 12.0,
                    8.0,
                    delta.z / horizontal * 12.0,
                )
        } else {
            target
        };
        let survive = self.common.world.random.next_int_bound(5).unwrap_or(0) > 0;
        let at = [p.position.x, p.position.y, p.position.z];
        let ctx = self.player_context(None);
        self.post_game_event(
            "minecraft:projectile_shoot",
            [start.x, start.y, start.z],
            ctx,
        );
        let id = self.take_entity_id();
        self.common.end.eyes.push(EyeOfEnder {
            id,
            position: start,
            previous: start,
            velocity: glam::DVec3::ZERO,
            target,
            life: 0,
            survive,
        });
        let r = self.common.world.random.next_float();
        let pitch = 0.33 + r * (0.5 - 0.33);
        self.server_play_sound("minecraft:entity.ender_eye.launch", at, 1.0, pitch);
        // CriteriaTriggers.USED_ENDER_EYE, against the feature's block.
        let (dx, dz) = (
            at[0] - f64::from(nearest.x * 16),
            at[2] - f64::from(nearest.z * 16),
        );
        self.trigger_used_ender_eye(dx * dx + dz * dz);
        self.consume_held();
        self.award_stat("used", "minecraft:ender_eye", 1);
        self.swing_arm();
    }
    /// EyeOfEnder.tick: fly, then drop (4 in 5) or shatter after 80 ticks.
    pub(in crate::game::server_side) fn tick_eyes(&mut self) {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let mut eyes = std::mem::take(&mut self.common.end.eyes);
        let mut shattered = Vec::new();
        eyes.retain_mut(|eye| {
            eye.previous = eye.position;
            let next = eye.position + eye.velocity;
            eye.velocity = EyeOfEnder::update_delta(eye.velocity, next, eye.target);
            eye.position = next;
            // Its move is filed at once (told with the dragon's step), also for one about to go.
            self.entity_section_step(eye.id, eye.previous.to_array(), next.to_array());
            eye.life += 1;
            if eye.life <= 80 {
                return true;
            }
            let at = eye.position.to_array();
            self.server_play_sound("minecraft:entity.ender_eye.death", at, 1.0, 1.0);
            if eye.survive {
                // new ItemEntity(level, x, y, z, item): random spread, 0.2 up.
                let vx = self.common.world.random.next_double() * 0.2 - 0.1;
                let vz = self.common.world.random.next_double() * 0.2 - 0.1;
                let stack = ItemStack {
                    item: "minecraft:ender_eye".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                };
                self.spawn_item(stack, at, [vx, 0.2, vz], 0);
            } else {
                shattered.push(at.map(|v| v.floor() as i32));
            }
            false
        });
        self.common.end.eyes = eyes;
        for pos in shattered {
            self.server_level_event_at(2003, pos, 0);
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
