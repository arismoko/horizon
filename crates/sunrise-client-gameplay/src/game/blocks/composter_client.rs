//! Client-side composter level event feedback (`ComposterBlock.handleFill`) and use check.
use crate::game::ClientGame;
use sunrise_core::{position::BlockPos, random::RandomSource};
use sunrise_items::items::ItemStack;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

fn composter_level(game: &crate::game::ClientGame<'_>, pos: [i32; 3]) -> Option<i32> {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    let s = game.pack.assets.registry.state(state)?;
    if &*s.name != "minecraft:composter" {
        return None;
    }
    s.properties.get("level")?.parse().ok()
}

impl ClientGame<'_> {
    /// `ComposterBlock.useItemOn` on a client level: compostable item and level below 8.
    pub(in crate::game::client_side) fn composter_takes(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        composter_level(self, pos).is_some_and(|level| level < 8)
            && held.is_some_and(|stack| {
                self.common
                    .res
                    .items()
                    .get(&stack.item)
                    .is_some_and(|i| i.compostable.is_some())
            })
    }
    /// `ComposterBlock.handleFill` (level event 1500) on the client: sound and 10 particles.
    pub(in crate::game) fn composter_fill_event(&mut self, pos: BlockPos, success: bool) {
        let center = [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z) + 0.5,
        ];
        let sound = if success {
            "minecraft:block.composter.fill_success"
        } else {
            "minecraft:block.composter.fill"
        };
        // `handleFill` is `playLocalSound`: capture would echo a sound packet.
        self.play_audio_unshared(sound, Some(center), 1.0, 1.0);
        // getShape(..).max(Y, 0.5, 0.5): the compost surface (SHAPES[8] = SHAPES[7]).
        let level = composter_level(self, [pos.x, pos.y, pos.z]).unwrap_or(0);
        let height = f64::from((1 + level.min(7) * 2).clamp(2, 16)) / 16.0 + 0.03125;
        let options = ParticleOptions::simple(ParticleType::Composter);
        for _ in 0..10 {
            let r = self.client.particles.0.random();
            let v = [
                r.next_gaussian() * 0.02,
                r.next_gaussian() * 0.02,
                r.next_gaussian() * 0.02,
            ];
            let p = [
                f64::from(pos.x) + 0.1875 + 0.625 * f64::from(r.next_float()),
                f64::from(pos.y) + height + f64::from(r.next_float()) * (1.0 - height),
                f64::from(pos.z) + 0.1875 + 0.625 * f64::from(r.next_float()),
            ];
            self.client.particles.0.add(&options, p, v);
        }
    }
}
