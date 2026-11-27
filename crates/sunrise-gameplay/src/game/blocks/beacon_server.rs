//! Server-side beacon ticking, effects, sounds, and advancement trigger.
use crate::game::ServerGame;
use sunrise_gameplay_common::game::beacon_common::{EFFECTS, effect_box_hits};
use sunrise_level::{
    containers::{Beacon, BlockEntity},
    effects::EffectInstance,
};

fn effect_id(name: &str) -> Option<&'static str> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    EFFECTS.iter().map(|(e, _)| *e).find(|e| *e == n)
}

impl ServerGame<'_> {
    /// `BeaconBlockEntity.tick` of the beacon at `key`.
    pub(in crate::game) fn tick_beacon_at(&mut self, key: (i32, i32, i32)) {
        let Some((lease, BlockEntity::Beacon(mut b))) = self.common.world.block_entities.take(&key)
        else {
            return;
        };
        let previous = b.levels;
        self.server_tick_beacon(key, &mut b);
        let current = b.levels;
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::Beacon(b));
        if (previous == 0) != (current == 0) {
            self.native_beacon_state_change(
                self.common.dim.id(),
                [key.0, key.1, key.2],
                current > 0,
            );
        }
    }

    fn server_tick_beacon(&mut self, key @ (x, y, z): (i32, i32, i32), b: &mut Beacon) {
        let (previous, ambient_ready, reached_surface) = self.advance_beacon_beam(key, b);
        if ambient_ready {
            self.beacon_effects([x, y, z], b);
            self.beacon_sound([x, y, z], "minecraft:block.beacon.ambient");
        }
        if reached_surface {
            let (was, is) = (previous > 0, b.levels > 0);
            if !was && is {
                self.beacon_sound([x, y, z], "minecraft:block.beacon.activate");
                let levels = b.levels;
                self.for_each_player(|g| g.beacon_constructed([x, y, z], levels));
            } else if was && !is {
                self.beacon_sound([x, y, z], "minecraft:block.beacon.deactivate");
            }
        }
    }

    /// `BeaconBlockEntity.tick` after activating: `CONSTRUCT_BEACON` in box.
    fn beacon_constructed(&mut self, p: [i32; 3], levels: i32) {
        let Some(bb) = self
            .common
            .player
            .as_ref()
            .map(|pl| pl.dimensions.bounds(pl.position))
        else {
            return;
        };
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let [x, y, z] = p.map(f64::from);
        if lo.x < x + 10.0
            && hi.x > x - 10.0
            && lo.y < y + 5.0
            && hi.y > y - 9.0
            && lo.z < z + 10.0
            && hi.z > z - 10.0
        {
            self.trigger_construct_beacon(levels);
        }
    }

    pub(in crate::game) fn beacon_sound(&mut self, p: [i32; 3], sound: &str) {
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ];
        self.server_play_sound(sound, at, 1.0, 1.0);
    }

    /// `BeaconBlockEntity.applyEffects`: every player in range.
    fn beacon_effects(&mut self, p: [i32; 3], b: &Beacon) {
        self.for_each_player(|g| g.beacon_effects_on_player(p, b));
    }

    fn beacon_effects_on_player(&mut self, p: [i32; 3], b: &Beacon) {
        let Some(primary) = b.primary.as_deref().and_then(effect_id) else {
            return;
        };
        let range = f64::from(b.levels * 10 + 10);
        let Some((pp, pl)) = self
            .common
            .player
            .as_ref()
            .map(|p| (p.position, p.dimensions))
        else {
            return;
        };
        let height = f64::from(self.common.world.top_y() - self.common.world.bottom_y());
        let bounds = pl.bounds(pp);
        if !effect_box_hits(
            p,
            range,
            height,
            [
                bounds.min_position().x,
                bounds.min_position().y,
                bounds.min_position().z,
            ],
            [
                bounds.max_position().x,
                bounds.max_position().y,
                bounds.max_position().z,
            ],
        ) {
            return;
        }
        let secondary = b.secondary.as_deref().and_then(effect_id);
        let amplifier = i32::from(b.levels >= 4 && secondary == Some(primary));
        let duration = (9 + b.levels * 2) * 20;
        let ambient = |id, amp| EffectInstance {
            ambient: true,
            ..EffectInstance::new(id, duration, amp)
        };
        self.add_player_effect(ambient(primary, amplifier));
        if let Some(s) = secondary.filter(|&s| b.levels >= 4 && s != primary) {
            self.add_player_effect(ambient(s, 0));
        }
    }
}

use sunrise_gameplay_common::game::beacon_common::BeaconCommonGameExt1 as _;
