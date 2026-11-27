use crate::game::ClientGame;
use sunrise_client::{particles::ParticleLevel, potent_sulfur::unobstructed_block_count};
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::potent_sulfur_common::{
    GEYSER_CONTINUOUS_ACTIVE, GEYSER_ERUPTION_ACTIVE,
};
use sunrise_gameplay_common::potent_sulfur::{
    Geyser, PARTICLE_FREQUENCY_TICKS, SOUND_FREQUENCY_TICKS, launch_area,
};
use sunrise_level::particle_options::{ParticleOptions, ParticleType as T};
use sunrise_level::{block_entities::Pos, containers::BlockEntity};

impl ClientGame<'_> {
    /// `PotentSulfurBlock.getTicker` on the client, for the block entity at
    /// `pos`: a wet or dormant geyser releases gas clouds every second; an
    /// erupting or continuous one shows its plume every second (from its last
    /// eruption event), plays its sound every two, and pushes the local
    /// player.
    pub(in crate::game) fn client_tick_potent_sulfur(&mut self, pos: Pos) {
        let Some((_, geyser)) = self.common.potent_sulfur_state(pos) else {
            return;
        };
        let time = self.common.world.game_time;
        // PotentSulfurBlockEntity.setLevel.
        let eruption_tick = match self.common.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::PotentSulfur(sulfur)) => {
                if sulfur.eruption_tick == -1 {
                    sulfur.eruption_tick = time;
                }
                sulfur.eruption_tick
            }
            _ => return,
        };
        match geyser {
            Geyser::Dry => {}
            // CLIENT_NOXIOUS_GAS_TICKER.
            Geyser::Wet | Geyser::Dormant => {
                if time % PARTICLE_FREQUENCY_TICKS == 0
                    && let Some(source) = self.common.potent_sulfur_source(pos)
                {
                    self.spawn_noxious_gas_cloud_particle(source);
                }
            }
            // CLIENT_GEYSER_PLUME_TICKER, then LAUNCH_ENTITY_TICKER.
            Geyser::Erupting | Geyser::Continuous => {
                if let Some(source) = self.common.potent_sulfur_source(pos) {
                    let eruption_time = time - eruption_tick;
                    if eruption_time % PARTICLE_FREQUENCY_TICKS == 0 {
                        self.spawn_geyser_particle(pos, source);
                    }
                    if eruption_time % SOUND_FREQUENCY_TICKS == 0 {
                        let sound = if geyser == Geyser::Erupting {
                            GEYSER_ERUPTION_ACTIVE
                        } else {
                            GEYSER_CONTINUOUS_ACTIVE
                        };
                        let at = source.map(|c| f64::from(c) + 0.5);
                        // playLocalSound: the seed is the level random's next long; a host's
                        // guests play their own.
                        let seed = self.client.particles.0.level_random().next_long();
                        if self.client.remote.is_some() {
                            self.play_audio_seeded(sound, Some(at), 1.0, 1.0, seed);
                        } else {
                            self.play_audio_unshared(sound, Some(at), 1.0, 1.0);
                        }
                    }
                }
                self.launch_local_player_ticker(pos);
            }
        }
    }
    /// `PotentSulfurBlockEntity.LAUNCH_ENTITY_TICKER` on the client: the local
    /// player in the column, unless spectating, flying or riding.
    pub(in crate::game::client_side) fn launch_local_player_ticker(&mut self, pos: Pos) {
        let Some(source) = self.common.potent_sulfur_source(pos) else {
            return;
        };
        let water_blocks = source[1] - pos.1 - 1;
        let height = {
            let level = ParticleLevel {
                world: &self.common.world,
                facts: self.common.res.facts(),
                collision: self.common.collision,
                player: None,
                cached_chunk: std::cell::Cell::new(None),
            };
            unobstructed_block_count(&level, [pos.0, pos.1 + 1, pos.2], water_blocks)
        };
        let area = launch_area(pos, height);
        let (spectator, riding) = (self.common.spectator(), self.common.riding.is_some());
        let Some(p) = self.common.player.as_mut() else {
            return;
        };
        if spectator || p.health.dead_or_dying() || !p.bounds().intersects(area) {
            return;
        }
        // Entity.checkFallDistanceAccumulation.
        if p.velocity.y > -0.5 && p.fall_distance > 1.0 {
            p.fall_distance = 1.0;
        }
        if !p.flying && !riding && p.velocity.y < f64::from(0.3f32) + f64::from(water_blocks) * 0.1
        {
            p.velocity.y += f64::from(0.2f32);
        }
    }
    /// `PotentSulfurBlockEntity.spawnGeyserParticle`: the eruption particle at
    /// the bottom of the source block, as high as the water column is deep.
    pub(in crate::game::client_side) fn spawn_geyser_particle(
        &mut self,
        sulfur: Pos,
        source: [i32; 3],
    ) {
        let water_blocks = source[1] - sulfur.1 - 1;
        self.client.particles.0.add(
            &ParticleOptions::geyser(T::Geyser, water_blocks),
            [
                f64::from(source[0]) + 0.5,
                f64::from(source[1]),
                f64::from(source[2]) + 0.5,
            ],
            [0.0; 3],
        );
    }
    /// `PotentSulfurBlockEntity.spawnNoxiousGasCloudParticle` at the centre of
    /// the source block.
    pub(in crate::game::client_side) fn spawn_noxious_gas_cloud_particle(
        &mut self,
        source: [i32; 3],
    ) {
        self.client.particles.0.add(
            &ParticleOptions::simple(T::NoxiousGasCloud),
            source.map(|c| f64::from(c) + 0.5),
            [0.0; 3],
        );
    }
}
